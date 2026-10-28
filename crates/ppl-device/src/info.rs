//! Reply types: raw exchanges, F8 probes and the parsed FA info.

use std::time::Duration;

use super::files::PrimeFile;
use super::guard::fnv64;

/// One raw report, recorded for the reverse-engineering log.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Exchange {
    pub outgoing: bool,
    pub bytes: Vec<u8>,
    pub at: Duration,
    /// Received while draining stale/late input; not part of any reply.
    pub drained: bool,
}

/// Result of an `F8` request.
#[derive(Clone, Debug)]
pub enum Probe {
    /// The calculator answered with an `F7` file.
    Present(PrimeFile),
    /// Nothing arrived within `waited`. This does **not** prove that the
    /// object is absent: the calculator may be busy, locked or slow.
    NoReply { waited: Duration },
}

impl Probe {
    pub fn file(&self) -> Option<&PrimeFile> {
        match self {
            Probe::Present(f) => Some(f),
            Probe::NoReply { .. } => None,
        }
    }
}

/// Parsed `FA` reply.
///
/// Field offsets follow BeatSkip/PrimeDev `HpInfos` and were confirmed on a
/// G1 (03f0:1541) running build 15157: counted back from the end of the
/// body, `build` is a u16 LE at −36, `version` 10 ASCII bytes at −32 and
/// `serial` 10 ASCII bytes at −16.
#[derive(Clone, Debug, Default)]
pub struct CalcInfo {
    pub build: Option<u32>,
    /// Boot/hardware version string, e.g. `V0.050.640`.
    pub version: Option<String>,
    pub serial: Option<String>,
    pub raw: Vec<u8>,
}

impl CalcInfo {
    pub fn parse(raw: &[u8]) -> CalcInfo {
        let body = &raw[6.min(raw.len())..];
        let n = body.len();
        let ascii = |from: usize, len: usize| -> Option<String> {
            let s = body.get(from..from + len)?;
            let t: String = s
                .iter()
                .take_while(|&&b| b != 0)
                .map(|&b| b as char)
                .collect();
            (!t.is_empty() && t.chars().all(|c| c.is_ascii_graphic())).then_some(t)
        };
        let build = (n >= 36)
            .then(|| u16::from_le_bytes([body[n - 36], body[n - 35]]) as u32)
            .filter(|&b| b > 0);
        CalcInfo {
            build,
            version: if n >= 32 { ascii(n - 32, 10) } else { None },
            serial: if n >= 16 { ascii(n - 16, 10) } else { None },
            raw: raw.to_vec(),
        }
    }

    /// Short stable fingerprint of the serial number for evidence files
    /// (the serial itself is not recorded).
    pub fn serial_fingerprint(&self) -> Option<String> {
        let s = self.serial.as_ref()?;
        // FNV-1a 64: not cryptographic, only avoids publishing the serial.
        Some(format!("{:016x}", fnv64(s.as_bytes())))
    }
}

/// Structured event recorded in optional diagnostic trace mode.
#[derive(Clone, Debug)]
pub struct TraceEvent {
    pub at: Duration,
    pub op_type: String,
    pub endpoint: String,
    pub requested_bytes: usize,
    pub actual_bytes: usize,
    pub status: String,
    pub cmd_id: Option<u8>,
    pub resp_id: Option<u8>,
    pub resp_len: Option<usize>,
    pub timeout_stage: Option<String>,
    pub elapsed: Duration,
    pub details: String,
}
