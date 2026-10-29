//! Bounded, rooted snapshots of observed value objects, never heap scans.
use super::{FirmwareValue, canonical_decimal, rational_decimal};
use crate::memory::GuestMemory;
use serde::{Deserialize, Serialize};

pub const MAX_LIST_DEPTH: usize = 8;
pub const MAX_LIST_ITEMS: usize = 4096;
pub const MAX_VALUE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepresentationNode {
    pub address: u32,
    /// One-based list element path; empty for the returned root.
    pub path: Vec<usize>,
    pub bytes: Vec<u8>,
    pub children: Vec<usize>,
    pub unavailable: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepresentationGraph {
    pub schema_version: u32,
    pub coherent: bool,
    pub complete: bool,
    pub captured_bytes: usize,
    pub logical_items: usize,
    pub nodes: Vec<RepresentationNode>,
    pub unavailable: Option<String>,
}

impl RepresentationGraph {
    pub(super) fn busy() -> Self {
        Self {
            schema_version: 1,
            coherent: false,
            complete: false,
            captured_bytes: 0,
            logical_items: 0,
            nodes: Vec::new(),
            unavailable: Some("coherent value snapshot busy at observation hook".into()),
        }
    }
}

struct Capture<'a> {
    mem: &'a GuestMemory,
    graph: RepresentationGraph,
    ancestors: Vec<u32>,
}

pub(super) fn capture(mem: &GuestMemory, address: u32) -> RepresentationGraph {
    let mut capture = Capture {
        mem,
        graph: RepresentationGraph {
            schema_version: 1,
            coherent: true,
            complete: true,
            captured_bytes: 0,
            logical_items: 0,
            nodes: Vec::new(),
            unavailable: None,
        },
        ancestors: Vec::new(),
    };
    capture.object(address, Vec::new());
    capture.graph
}

fn word(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| "incomplete value representation".into())
}

impl Capture<'_> {
    fn read_to(&mut self, index: usize, size: usize) -> Result<(), String> {
        let node = &self.graph.nodes[index];
        let additional = size.saturating_sub(node.bytes.len());
        if additional > MAX_VALUE_BYTES - self.graph.captured_bytes {
            return Err("value representation exceeds 1 MiB capture bound".into());
        }
        if additional == 0 {
            return Ok(());
        }
        let at = node
            .address
            .checked_add(node.bytes.len() as u32)
            .ok_or("value pointer overflow")?;
        let bytes = self
            .mem
            .read_bytes_alloc(at, additional)
            .map_err(|e| e.to_string())?;
        self.graph.captured_bytes += bytes.len();
        self.graph.nodes[index].bytes.extend(bytes);
        Ok(())
    }

    fn object(&mut self, address: u32, path: Vec<usize>) -> usize {
        let index = self.graph.nodes.len();
        self.graph.nodes.push(RepresentationNode {
            address,
            path,
            bytes: Vec::new(),
            children: Vec::new(),
            unavailable: None,
        });
        if let Err(reason) = self.object_inner(index) {
            self.graph.complete = false;
            self.graph.nodes[index].unavailable = Some(reason);
        }
        index
    }

    fn object_inner(&mut self, index: usize) -> Result<(), String> {
        let address = self.graph.nodes[index].address;
        if address == 0 || address & 3 != 0 {
            return Err("null or unaligned value pointer".into());
        }
        if self.ancestors.contains(&address) {
            return Err("cyclic list reference".into());
        }
        self.read_to(index, 4)?;
        let header = word(&self.graph.nodes[index].bytes, 0)?;
        let (tag, sub) = ((header >> 16) as u8, (header >> 24) as u8);
        let size = match tag & 15 {
            0 => 16,
            15 if matches!(sub, 0 | 1) => 24,
            2 => {
                self.read_to(index, 8)?;
                let length = word(&self.graph.nodes[index].bytes, 4)? as usize;
                if length > 65536 {
                    return Err("guest string exceeds capture bound".into());
                }
                8 + (length + 1) * 2
            }
            5 => 8,
            // Candidate layout from the separate runtime-parity branch.
            // Capturing this shape does not establish that it is decoded.
            6 if matches!(sub, 0 | 1 | 0x20) => {
                self.read_to(index, 8)?;
                if self.graph.nodes[index].path.len() >= MAX_LIST_DEPTH {
                    return Err("guest list nesting exceeds capture bound".into());
                }
                let count = word(&self.graph.nodes[index].bytes, 4)? as usize;
                if count > MAX_LIST_ITEMS - self.graph.logical_items {
                    return Err("guest list exceeds logical item capture bound".into());
                }
                self.graph.logical_items += count;
                let size = count
                    .checked_mul(4)
                    .and_then(|n| n.checked_add(8))
                    .ok_or("list table length overflow")?;
                self.read_to(index, size)?;
                self.ancestors.push(address);
                for i in 0..count {
                    let child_address = word(&self.graph.nodes[index].bytes, 8 + i * 4)?;
                    let mut path = self.graph.nodes[index].path.clone();
                    path.push(i + 1);
                    let child = self.object(child_address, path);
                    self.graph.nodes[index].children.push(child);
                    if self.graph.captured_bytes >= MAX_VALUE_BYTES {
                        break;
                    }
                }
                self.ancestors.pop();
                if self.graph.nodes[index].children.len() != count {
                    return Err("value representation exceeds 1 MiB capture bound".into());
                }
                return Ok(());
            }
            _ => return Err(format!("unverified runtime value tag {tag:#x}/{sub:#x}")),
        };
        self.read_to(index, size)
    }
}

/// Decode only the retained graph, so the same bytes can be independently
/// replayed after the guest has exited. Capture and logical output share a
/// byte budget; shared objects are charged for every logical occurrence.
pub(super) fn decode(graph: &RepresentationGraph) -> Result<FirmwareValue, String> {
    if !graph.coherent {
        return Err(graph
            .unavailable
            .clone()
            .unwrap_or_else(|| "incoherent value capture".into()));
    }
    if graph.schema_version != 1 || graph.nodes.is_empty() || graph.nodes.len() > MAX_LIST_ITEMS + 1
    {
        return Err("invalid value graph schema or node count".into());
    }
    let bytes = graph
        .nodes
        .iter()
        .try_fold(0usize, |total, node| total.checked_add(node.bytes.len()))
        .ok_or("capture length overflow")?;
    if bytes != graph.captured_bytes || bytes > MAX_VALUE_BYTES {
        return Err("value representation exceeds 1 MiB capture bound".into());
    }
    if !graph.complete {
        if let Some(node) = graph.nodes.iter().find(|n| n.unavailable.is_some()) {
            let prefix = node
                .path
                .last()
                .map(|i| format!("list element {i}: "))
                .unwrap_or_default();
            return Err(format!(
                "{prefix}{}: {}",
                path_name(&node.path),
                node.unavailable.as_deref().unwrap()
            ));
        }
        return Err("incomplete value representation graph".into());
    }
    let mut decoder = Decoder {
        graph,
        remaining_bytes: MAX_VALUE_BYTES - bytes,
        remaining_items: MAX_LIST_ITEMS,
        ancestors: Vec::new(),
        visited: std::collections::HashSet::new(),
    };
    let value = decoder.object(0, &[])?;
    if decoder.visited.len() != graph.nodes.len() {
        return Err("value graph contains unrooted objects".into());
    }
    if MAX_LIST_ITEMS - decoder.remaining_items != graph.logical_items {
        return Err("value graph logical item count disagrees with its tables".into());
    }
    Ok(value)
}

fn path_name(path: &[usize]) -> String {
    format!(
        "return{}",
        path.iter().map(|i| format!("[{i}]")).collect::<String>()
    )
}

struct Decoder<'a> {
    graph: &'a RepresentationGraph,
    remaining_bytes: usize,
    remaining_items: usize,
    ancestors: Vec<u32>,
    visited: std::collections::HashSet<usize>,
}

impl Decoder<'_> {
    fn charge(&mut self, bytes: usize) -> Result<(), String> {
        self.remaining_bytes = self
            .remaining_bytes
            .checked_sub(bytes)
            .ok_or("value capture and decoded output exceed 1 MiB bound")?;
        Ok(())
    }

    fn object(&mut self, index: usize, path: &[usize]) -> Result<FirmwareValue, String> {
        self.object_inner(index, path)
            .map_err(|reason| format!("{}: {reason}", path_name(path)))
    }

    fn object_inner(&mut self, index: usize, path: &[usize]) -> Result<FirmwareValue, String> {
        let node = self
            .graph
            .nodes
            .get(index)
            .ok_or("missing child representation")?;
        if node.path != path || node.address == 0 || node.address & 3 != 0 {
            return Err("invalid object path or pointer alignment".into());
        }
        if self.ancestors.contains(&node.address) || !self.visited.insert(index) {
            return Err("cyclic or reused graph node".into());
        }
        let bytes = &node.bytes;
        let header = word(bytes, 0)?;
        let (tag, sub) = ((header >> 16) as u8, (header >> 24) as u8);
        let kind = tag & 15;
        let value = match kind {
            0 if bytes.len() == 16 => decimal_real(bytes)?,
            15 if sub == 0 && bytes.len() == 24 => decimal_real(&bytes[8..])?,
            15 if sub == 1 && bytes.len() == 24 => {
                let sign = bytes[4];
                if sign > 1 {
                    return Err("unrecognized rational sign".into());
                }
                let numerator = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                let denominator = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
                if denominator == 0 {
                    return Err("zero rational denominator".into());
                }
                if let Some(text) = rational_decimal(numerator, denominator, sign != 0) {
                    FirmwareValue::Real {
                        value: canonical_decimal(&text)?,
                    }
                } else {
                    FirmwareValue::Rational {
                        negative: sign != 0,
                        numerator: numerator.to_string(),
                        denominator: denominator.to_string(),
                    }
                }
            }
            2 => {
                let count = word(bytes, 4)? as usize;
                if count > 65536 || bytes.len() != 8 + (count + 1) * 2 {
                    return Err("guest string length mismatch".into());
                }
                let units: Vec<_> = bytes[8..]
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                if units.last() != Some(&0) || units[..count].contains(&0) {
                    return Err("guest string terminator or length mismatch".into());
                }
                FirmwareValue::String {
                    value: String::from_utf16(&units[..count])
                        .map_err(|_| "invalid UTF-16 guest string")?,
                }
            }
            5 if bytes.len() == 8 => FirmwareValue::Error { raw_code: sub },
            // Independently observed in this checkout's capture-only controls:
            // Subtypes 0x00 and 0x20 are supported by retained captures;
            // 0x01 is supported by the G1 2.4.2 runtime-parity controls.
            6 if matches!(sub, 0 | 1 | 0x20) => {
                if path.len() >= MAX_LIST_DEPTH {
                    return Err("guest list nesting exceeds capture bound".into());
                }
                let count = word(bytes, 4)? as usize;
                if count > self.remaining_items
                    || bytes.len() != 8 + count * 4
                    || node.children.len() != count
                {
                    return Err("invalid list table or logical item bound".into());
                }
                self.remaining_items -= count;
                self.charge(br#"{"type":"list","value":[]}"#.len() + count.saturating_sub(1))?;
                self.ancestors.push(node.address);
                let mut items = Vec::with_capacity(count);
                for (i, &child) in node.children.iter().enumerate() {
                    let child_node = self.graph.nodes.get(child).ok_or("missing list child")?;
                    if child_node.address != word(bytes, 8 + i * 4)? {
                        return Err("list pointer and captured child disagree".into());
                    }
                    let mut child_path = path.to_vec();
                    child_path.push(i + 1);
                    items.push(self.object(child, &child_path)?);
                }
                self.ancestors.pop();
                return Ok(FirmwareValue::List { items });
            }
            _ => return Err(format!("unverified runtime value tag {tag:#x}/{sub:#x}")),
        };
        if !node.children.is_empty() {
            return Err("scalar object has unexpected children".into());
        }
        self.charge(serde_json::to_vec(&value).map_err(|e| e.to_string())?.len())?;
        Ok(value)
    }
}

fn decimal_real(bytes: &[u8]) -> Result<FirmwareValue, String> {
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
        let digit = ((word >> (12 + n * 4)) & 15) as u8;
        if digit > 9 {
            return Err("non-BCD decimal mantissa".into());
        }
        digits.push((b'0' + digit) as char);
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
