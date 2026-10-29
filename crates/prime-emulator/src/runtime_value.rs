//! Checked decoding of observed G1 runtime values and lossless numeric input.
//! This is a representation codec, not a PPL evaluator.
use crate::{
    keys::{self, Key},
    memory::GuestMemory,
};
use serde::{Deserialize, Serialize};

mod representation;
pub use representation::{
    MAX_LIST_DEPTH, MAX_LIST_ITEMS, MAX_VALUE_BYTES, RepresentationGraph, RepresentationNode,
};

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
    List {
        #[serde(rename = "value", alias = "items")]
        items: Vec<FirmwareValue>,
    },
}

impl FirmwareValue {
    /// Readable host status text; structural comparisons use typed values.
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
            Self::String { value } => serde_json::to_string(value).expect("serializable string"),
            Self::Error { raw_code } => format!("error {raw_code}"),
            Self::List { items } => format!(
                "{{{}}}",
                items
                    .iter()
                    .map(|item| item.render(false))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueEvidence {
    pub address: u32,
    pub tag: Option<u8>,
    pub subtype: Option<u8>,
    /// Bounded representation bytes; never an arbitrary heap dump.
    pub bytes: Vec<u8>,
    pub value: Option<FirmwareValue>,
    pub unavailable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representation: Option<RepresentationGraph>,
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

/// Snapshot an observed object without changing guest values or reference counts.
/// The existing gate also supports callers already inside an observing hook.
pub fn decode(mem: &GuestMemory, address: u32) -> ValueEvidence {
    let Some(_snapshot) = mem.snapshot_gate.try_snapshot_from_hook() else {
        let mut evidence = decode_representation(RepresentationGraph::busy());
        evidence.address = address;
        return evidence;
    };
    decode_representation(representation::capture(mem, address))
}

pub fn decode_return(mem: &GuestMemory, address: u32) -> ValueEvidence {
    decode(mem, address)
}

/// Replay retained representation bytes without running the firmware again.
/// This does not upgrade historical results which lack representation bytes.
pub fn decode_representation(graph: RepresentationGraph) -> ValueEvidence {
    let root = graph.nodes.first();
    let address = root.map_or(0, |n| n.address);
    let header = root
        .and_then(|n| n.bytes.get(..4))
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let bytes = root
        .map(|n| n.bytes[..n.bytes.len().min(24)].to_vec())
        .unwrap_or_default();
    let result = representation::decode(&graph);
    ValueEvidence {
        address,
        tag: header.map(|h| (h >> 16) as u8),
        subtype: header.map(|h| (h >> 24) as u8),
        bytes,
        value: result.as_ref().ok().cloned(),
        unavailable: result.as_ref().err().cloned(),
        probe: if result.is_err() {
            graph
                .nodes
                .iter()
                .map(|n| ProbeBlock {
                    address: n.address,
                    bytes: n.bytes.clone(),
                })
                .collect()
        } else {
            Vec::new()
        },
        representation: Some(graph),
    }
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
