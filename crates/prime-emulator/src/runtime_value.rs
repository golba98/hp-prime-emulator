//! Checked decoding of observed G1 runtime values and lossless numeric input.
//! This is a representation codec, not a PPL evaluator.
use crate::{
    compiler::read_utf16,
    keys::{self, Key},
    memory::GuestMemory,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FirmwareValue {
    Real {
        value: String,
    },
    Rational {
        negative: bool,
        numerator: String,
        denominator: String,
    },
    String {
        value: String,
    },
    /// Observed guest error enum, not a documented SDK error namespace.
    Error {
        raw_code: u8,
    },
    /// Kind-6 list: element count at +4, element object pointers from +8.
    /// Layout derived from G1 2.4.2 captures of `{}`, `{5}`, `{1,2,3}`,
    /// `{-1.5,1E200,0.001}`, `{"a",2}`, `{{1},2}` and a subtype-0x01
    /// `{0,100,50}` whose first element lives in firmware ROM.
    List {
        items: Vec<FirmwareValue>,
    },
}

impl FirmwareValue {
    /// Plain-text rendering in the Home history's element order and
    /// separators (`{"a", 2}`), using ASCII `e`/`-` instead of the
    /// calculator's small-E and raised minus glyphs.
    pub fn display(&self) -> String {
        self.render(true)
    }
    fn render(&self, top: bool) -> String {
        match self {
            Self::Real { value } => value.clone(),
            Self::Rational {
                negative,
                numerator,
                denominator,
            } => format!(
                "{}{numerator}/{denominator}",
                if *negative { "-" } else { "" }
            ),
            Self::String { value } if top => value.clone(),
            Self::String { value } => format!("\"{value}\""),
            Self::Error { raw_code } => format!("error {raw_code}"),
            Self::List { items } => format!(
                "{{{}}}",
                items
                    .iter()
                    .map(|i| i.render(false))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// Bounds for decoding nested guest lists; larger values stay undecoded.
pub const MAX_LIST_DEPTH: usize = 8;
pub const MAX_LIST_ITEMS: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueEvidence {
    pub address: u32,
    pub tag: Option<u8>,
    pub subtype: Option<u8>,
    /// Bounded representation bytes; never an arbitrary heap dump.
    pub bytes: Vec<u8>,
    pub value: Option<FirmwareValue>,
    pub unavailable: Option<String>,
    /// Bounded blocks of an undecoded object and the blocks it points to,
    /// recorded so new layouts are derived from guest bytes (empty when the
    /// value decoded; `PRIME_VALUE_PROBE=0` disables it).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub probe: Vec<ProbeBlock>,
}

/// What the firmware returned, as far as the host could observe it. A
/// returned object the host cannot decode is reported, never invented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReturnRepresentation {
    Decoded,
    Undecoded {
        tag: Option<u8>,
        subtype: Option<u8>,
        reason: String,
    },
    /// No returned object, or the result was an error object.
    None,
}
impl ReturnRepresentation {
    pub fn of(result: Option<&ValueEvidence>) -> Self {
        match result {
            Some(ValueEvidence {
                value: Some(FirmwareValue::Error { .. }),
                ..
            })
            | None => Self::None,
            Some(ValueEvidence { value: Some(_), .. }) => Self::Decoded,
            Some(v) => Self::Undecoded {
                tag: v.tag,
                subtype: v.subtype,
                reason: v
                    .unavailable
                    .clone()
                    .unwrap_or_else(|| "unknown representation".into()),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeBlock {
    pub address: u32,
    pub bytes: Vec<u8>,
}

const PROBE_OBJECT: usize = 64;
const PROBE_CHILD: usize = 48;
const PROBE_CHILDREN: usize = 8;

fn probe(mem: &GuestMemory, address: u32) -> Vec<ProbeBlock> {
    if std::env::var_os("PRIME_VALUE_PROBE").is_some_and(|v| v == "0") {
        return Vec::new();
    }
    let Ok(object) = mem.read_bytes_alloc(address, PROBE_OBJECT) else {
        return Vec::new();
    };
    let mut blocks = vec![ProbeBlock {
        address,
        bytes: object.clone(),
    }];
    for word in object.chunks_exact(4).skip(1) {
        let p = u32::from_le_bytes(word.try_into().unwrap());
        if blocks.len() > PROBE_CHILDREN || p & 3 != 0 || blocks.iter().any(|b| b.address == p) {
            continue;
        }
        if let Ok(bytes) = mem.read_bytes_alloc(p, PROBE_CHILD) {
            blocks.push(ProbeBlock { address: p, bytes });
        }
    }
    blocks
}

pub fn decode(mem: &GuestMemory, address: u32) -> ValueEvidence {
    let mut budget = MAX_LIST_ITEMS;
    let result = (|| -> Result<(u8, u8, Vec<u8>, FirmwareValue), String> {
        let (tag, sub, size, value) = decode_object(mem, address, 0, &mut budget)?;
        let bytes = mem
            .read_bytes_alloc(address, size)
            .map_err(|e| e.to_string())?;
        Ok((tag, sub, bytes, value))
    })();
    match result {
        Ok((tag, subtype, bytes, value)) => ValueEvidence {
            address,
            tag: Some(tag),
            subtype: Some(subtype),
            bytes,
            value: Some(value),
            unavailable: None,
            probe: Vec::new(),
        },
        Err(e) => {
            let header = (address != 0 && address & 3 == 0)
                .then(|| mem.read_u32(address).ok())
                .flatten();
            ValueEvidence {
                address,
                tag: header.map(|h| (h >> 16) as u8),
                subtype: header.map(|h| (h >> 24) as u8),
                bytes: Vec::new(),
                value: None,
                unavailable: Some(e),
                probe: probe(mem, address),
            }
        }
    }
}

/// Decode one object; returns its tag, subtype, header size and value.
fn decode_object(
    mem: &GuestMemory,
    address: u32,
    depth: usize,
    budget: &mut usize,
) -> Result<(u8, u8, usize, FirmwareValue), String> {
    if address == 0 || address & 3 != 0 || address.checked_add(32).is_none() {
        return Err("null or unaligned value pointer".into());
    }
    let header = mem.read_u32(address).map_err(|e| e.to_string())?;
    let tag = (header >> 16) as u8;
    let sub = (header >> 24) as u8;
    let kind = tag & 15;
    let (size, value) = match kind {
        0 => (16, decimal_real(mem, address)?),
        15 if sub == 0 => (
            24,
            decimal_real(mem, address.checked_add(8).ok_or("value overflow")?)?,
        ),
        15 if sub == 1 => {
            let sign = (mem.read_u32(address + 4).map_err(|e| e.to_string())? & 255) as u8;
            if sign > 1 {
                return Err("unrecognized rational sign".into());
            }
            let bytes = mem
                .read_bytes_alloc(address + 8, 16)
                .map_err(|e| e.to_string())?;
            let numerator = u64::from_le_bytes(bytes[..8].try_into().unwrap());
            let denominator = u64::from_le_bytes(bytes[8..].try_into().unwrap());
            if denominator == 0 {
                return Err("zero rational denominator".into());
            }
            let value = if let Some(text) = rational_decimal(numerator, denominator, sign != 0) {
                FirmwareValue::Real {
                    value: canonical_decimal(&text)?,
                }
            } else {
                FirmwareValue::Rational {
                    negative: sign != 0,
                    numerator: numerator.to_string(),
                    denominator: denominator.to_string(),
                }
            };
            (24, value)
        }
        2 => {
            let length = mem.read_u32(address + 4).map_err(|e| e.to_string())? as usize;
            if length > 65536 {
                return Err("guest string exceeds capture bound".into());
            }
            let value = read_utf16(mem, address + 8, length + 1)?;
            if value.encode_utf16().count() != length {
                return Err("guest string length mismatch".into());
            }
            (8, FirmwareValue::String { value })
        }
        5 => (8, FirmwareValue::Error { raw_code: sub }),
        // Only subtypes observed with this layout are accepted (0x01 from a
        // live-session `WAIT(-1)` result list; the byte behaves as flags).
        6 if matches!(sub, 0x00 | 0x01 | 0x20) => {
            if depth >= MAX_LIST_DEPTH {
                return Err("guest list nesting exceeds capture bound".into());
            }
            let count = mem.read_u32(address + 4).map_err(|e| e.to_string())? as usize;
            if count > *budget {
                return Err("guest list exceeds capture bound".into());
            }
            *budget -= count;
            let mut items = Vec::with_capacity(count);
            for i in 0..count {
                let slot = address
                    .checked_add(8 + 4 * i as u32)
                    .ok_or("value overflow")?;
                let element = mem.read_u32(slot).map_err(|e| e.to_string())?;
                let (_, _, _, item) = decode_object(mem, element, depth + 1, budget)
                    .map_err(|e| format!("list element {}: {e}", i + 1))?;
                items.push(item);
            }
            (8 + 4 * count, FirmwareValue::List { items })
        }
        _ => return Err(format!("unverified runtime value tag {tag:#x}/{sub:#x}")),
    };
    Ok((tag, sub, size, value))
}

fn decimal_real(mem: &GuestMemory, p: u32) -> Result<FirmwareValue, String> {
    let bytes = mem.read_bytes_alloc(p, 16).map_err(|e| e.to_string())?;
    let sign = bytes[3] as i8;
    if ![-1, 0, 1].contains(&sign) {
        return Err("unverified decimal sign/special value".into());
    }
    let exp = i32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if !(-499..=499).contains(&exp) {
        return Err("unverified decimal exponent/special value".into());
    }
    let word = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    if word & 0xfff != 0 || word >> 60 != 0 {
        return Err("unverified decimal mantissa flags".into());
    }
    if sign == 0 && word != 0 {
        return Err("inconsistent decimal zero sign".into());
    }
    let mut digits = String::new();
    for n in (0..12).rev() {
        let d = ((word >> (12 + n * 4)) & 15) as u8;
        if d > 9 {
            return Err("non-BCD decimal mantissa".into());
        }
        digits.push((b'0' + d) as char);
    }
    let text = format!(
        "{}{}.{}e{exp}",
        if sign < 0 { "-" } else { "" },
        &digits[..1],
        &digits[1..]
    );
    Ok(FirmwareValue::Real {
        value: canonical_decimal(&text)?,
    })
}

fn rational_decimal(n: u64, d: u64, negative: bool) -> Option<String> {
    let mut text = format!("{}{}", if negative && n != 0 { "-" } else { "" }, n / d);
    let mut remainder = (n % d) as u128;
    if remainder != 0 {
        text.push('.');
    }
    // A terminating u64-denominator decimal needs at most 63 fractional digits.
    for _ in 0..64 {
        if remainder == 0 {
            return Some(text);
        }
        remainder *= 10;
        text.push((b'0' + (remainder / d as u128) as u8) as char);
        remainder %= d as u128;
    }
    (remainder == 0).then_some(text)
}

/// Canonical decimal spelling, without binary floating point or rounding.
/// JSON/argument validation permits finite decimal literals only.
pub fn canonical_decimal(input: &str) -> Result<String, String> {
    if input.len() > 1024 || input.is_empty() {
        return Err("numeric literal is empty or exceeds 1024 bytes".into());
    }
    let b = input.as_bytes();
    let mut i = 0;
    let negative = b[0] == b'-';
    if negative {
        i += 1;
    }
    let start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == start {
        return Err("expected decimal integer part".into());
    }
    let mut digits = input[start..i].to_owned();
    let mut fractional = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return Err("expected fractional digits".into());
        }
        fractional = (i - start) as i32;
        digits.push_str(&input[start..i]);
    }
    let mut exponent = 0i32;
    if i < b.len() && matches!(b[i], b'e' | b'E') {
        i += 1;
        let start = i;
        if i < b.len() && matches!(b[i], b'+' | b'-') {
            i += 1;
        }
        let d = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if d == i {
            return Err("expected exponent digits".into());
        }
        exponent = input[start..i]
            .parse::<i32>()
            .map_err(|_| "exponent is out of bounds")?;
    }
    if i != b.len() {
        return Err("unsupported numeric literal".into());
    }
    digits = digits.trim_start_matches('0').to_owned();
    if digits.is_empty() {
        return Ok("0".into());
    }
    let mut power = exponent
        .checked_sub(fractional)
        .ok_or("exponent overflow")?;
    while digits.ends_with('0') {
        digits.pop();
        power = power.checked_add(1).ok_or("exponent overflow")?;
    }
    let leading = power
        .checked_add(digits.len() as i32 - 1)
        .ok_or("exponent overflow")?;
    if !(-499..=499).contains(&leading) {
        return Err("numeric magnitude is outside the verified G1 range".into());
    }
    let sign = if negative { "-" } else { "" };
    let value = if (-6..=15).contains(&leading) {
        let point = digits.len() as i32 + power;
        if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point >= digits.len() as i32 {
            format!(
                "{}{}",
                digits,
                "0".repeat((point - digits.len() as i32) as usize)
            )
        } else {
            format!(
                "{}.{}",
                &digits[..point as usize],
                &digits[point as usize..]
            )
        }
    } else if digits.len() == 1 {
        format!("{digits}e{leading}")
    } else {
        format!("{}.{}e{leading}", &digits[..1], &digits[1..])
    };
    Ok(format!("{sign}{value}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumericArgument {
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
}

pub fn parse_arguments(text: &str) -> Result<Vec<NumericArgument>, String> {
    let values: Vec<Box<serde_json::value::RawValue>> =
        serde_json::from_str(text).map_err(|e| e.to_string())?;
    if values.len() > 31 {
        return Err("at most 31 numeric arguments are supported".into());
    }
    values
        .into_iter()
        .map(|v| {
            let text = v.get();
            let literal = if text.starts_with('{') {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Tagged {
                    #[serde(rename = "type")]
                    kind: String,
                    value: String,
                }
                let t: Tagged = serde_json::from_str(text).map_err(|e| e.to_string())?;
                if t.kind != "real" {
                    return Err(
                        "only real numeric arguments have verified guest input encoding".into(),
                    );
                }
                t.value
            } else {
                text.to_owned()
            };
            Ok(NumericArgument {
                kind: "real".into(),
                value: canonical_decimal(&literal)?,
            })
        })
        .collect()
}

/// Original calculator input, never a guest-memory injection. Zero-argument
/// functions are invoked as bare names in the Home evaluator.
pub fn invocation_keys(entry: &str, args: &[NumericArgument]) -> Result<Vec<Key>, String> {
    let mut script = vec!["HOME".to_owned(), "ESC".to_owned()];
    if entry.is_empty() || entry.len() > 64 {
        return Err("entry name is empty or exceeds the verified input bound".into());
    }
    for c in entry.chars() {
        if c.is_ascii_alphabetic() {
            script.push("ALPHA".into());
            if c.is_ascii_lowercase() {
                script.push("SHIFT".into());
            }
            script.push(format!("0x{:02X}", c.to_ascii_uppercase() as u32));
        } else if c.is_ascii_digit() {
            script.push(c.to_string());
        } else if c == '_' {
            script.extend(["SHIFT".into(), "0x20".into()]);
        } else {
            return Err("entry requires an unverified character input encoding".into());
        }
    }
    if !args.is_empty() {
        script.push("0x4E".into());
        for (i, arg) in args.iter().enumerate() {
            if arg.kind != "real" {
                return Err("unsupported argument type".into());
            }
            if i > 0 {
                script.push("0x4F".into());
            }
            for c in canonical_decimal(&arg.value)?.chars() {
                script.push(match c {
                    'e' | 'E' => "0x50".into(),
                    '-' => "0x4D".into(),
                    _ => c.to_string(),
                });
            }
        }
        script.push("RIGHT".into());
    }
    script.push("ENTER".into());
    keys::parse_keys(&script.join(","))
}
