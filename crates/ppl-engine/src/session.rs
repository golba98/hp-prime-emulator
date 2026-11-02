//! Versioned local calculator-session transport. No GUI, USB, or Unicorn dependency.
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
/// 2: `Lifecycle`, `Unsupported` and `Import`. 3: `PowerDown`, `Sleeping`.
/// 4: atomic desktop `Text` packets, translated using observed firmware state.
/// 5: explicit managed-program capabilities, independent of calculator availability.
/// 6: exact named programs, structured runtime evidence, scoped input and persistence checkpoints.
pub const VERSION: u32 = 6;
pub const MAX_PACKET: usize = 4 * 1024 * 1024;
pub const FRAME_PIXELS: usize = 320 * 240;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Bounded, prevalidated desktop text; never interprets newlines as Enter.
    Text {
        text: String,
        #[serde(default)]
        paste: bool,
    },
    Key {
        code: u16,
        down: bool,
    },
    Touch {
        status: u32,
        x: u16,
        y: u16,
    },
    ReleaseAll,
    Script {
        keys: String,
    },
    LoadSource {
        request: u64,
        source: String,
    },
    FrameRequest,
    Compile {
        request: u64,
        source: String,
    },
    /// Check `source` with the original firmware compiler called directly
    /// (no calculator UI driving); the verdict comes back in
    /// `Event::Result.check`.
    DirectCheck {
        request: u64,
        source: String,
    },
    Run {
        request: u64,
        source: String,
        entry: Option<String>,
        arguments: String,
    },
    /// Store `source` as a new calculator program `name` through the
    /// firmware's own `Programs(name):=` assignment. Refused when a program
    /// of that name exists; never replaces one.
    Import {
        request: u64,
        name: String,
        source: String,
    },
    /// Session-owned named source assignment. Replacing requires exact old source identity.
    StoreProgram {
        request: u64,
        name: String,
        source: String,
        expected_previous_sha256: Option<String>,
    },
    /// Execute a stored exact revision; never reassigns or checks its source.
    RunStored {
        request: u64,
        name: String,
        source_sha256: String,
        entry: String,
        arguments: String,
    },
    /// Explicit opt-in firmware OFF persistence checkpoint; not a live CPU image.
    Checkpoint {
        id: u64,
        wake: bool,
    },
    ScenarioInput {
        request: u64,
        run_request: u64,
        input: CalculatorInput,
    },
    ReleaseInput {
        request: u64,
    },
    Stop,
    /// Switch the calculator off through the firmware's own OFF (which
    /// saves its state), then end the session. A running program is
    /// stopped first.
    PowerDown,
    /// End the session at once, without the firmware's OFF.
    Shutdown,
}
/// Physical calculator events for an explicitly active managed invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CalculatorInput {
    Key { physical: u8, down: bool },
    Touch { status: u32, x: u16, y: u16 },
}
/// Firmware scan codes from the existing GUI mapping, shared without a GUI dependency.
pub const PHYSICAL_DEVICE_CODES: [u16; 51] = [
    0x00B1, 0x0091, 0x0003, 0x0095, 0x0001, 0xE047, 0x00B2, 0x0002, 0x0004, 0x00B4, 0x00B5, 0x00B3,
    0x0005, 0x0093, 0x0041, 0x0042, 0x0043, 0x0044, 0x0045, 0x000C, 0x0046, 0x0047, 0x0048, 0x0049,
    0x004A, 0x004B, 0x004C, 0x004D, 0x004E, 0x004F, 0x000D, 0x0050, 0x0051, 0x0052, 0x0053, 0x0054,
    0x00B6, 0x0055, 0x0056, 0x0057, 0x0058, 0x008B, 0x0059, 0x005A, 0x0033, 0x00B7, 0x0083, 0x0030,
    0x00B8, 0x0020, 0x00B9,
];
/// The live calculator's state as the session observes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    /// The firmware is starting; its UI has not yet waited for input.
    Booting,
    /// The firmware UI is idle, waiting for input.
    Ready,
    /// A Stop is breaking a running program.
    Stopping,
    /// The firmware switched off (`SysPowerOff`) after saving; ON wakes it.
    Sleeping,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Checkpoint {
        id: u64,
        result: crate::checkpoint::CheckpointResult,
    },
    Hello {
        version: u32,
    },
    Capabilities {
        managed_programs: bool,
        reason: Option<String>,
    },
    Phase {
        phase: String,
    },
    Status {
        message: String,
    },
    Result {
        request: u64,
        success: bool,
        message: String,
        /// The firmware compiler's verdict on the request's exact source,
        /// when one was observed. `None` means no verdict (cancelled,
        /// busy, infrastructure failure), never acceptance or rejection.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        check: Option<CheckVerdict>,
    },
    /// Correlated terminal runtime evidence, independent of human status text.
    Execution {
        request: u64,
        program: String,
        source_sha256: String,
        observation: Box<ExecutionObservation>,
    },
    ExecutionStarted {
        request: u64,
        source_sha256: String,
        entry: String,
    },
    Lifecycle {
        state: Lifecycle,
    },
    /// The firmware called an OS service the host does not implement, from
    /// a call site not reported before; the host returned 0 in its place.
    Unsupported {
        id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        caller_pc: u32,
    },
    Error {
        message: String,
    },
    Closed {
        message: String,
    },
}
/// Structured evidence from actual guest execution. Raw representation remains available.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionObservation {
    pub status: String,
    pub binding: String,
    pub entry: String,
    pub body_entered: bool,
    pub completed: bool,
    pub return_value: Option<serde_json::Value>,
    pub return_representation: serde_json::Value,
    pub output: Vec<crate::OutputEvent>,
    pub diagnostics: Vec<crate::Diagnostic>,
    pub runtime: serde_json::Value,
    pub image: Option<ExecutionImage>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}
/// An original-firmware Check verdict observed in the live session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckVerdict {
    pub accepted: bool,
    /// SHA-256 of the exact source the firmware compiler parsed.
    pub source_sha256: String,
    /// The guest's diagnostic for a rejection (one, the first error);
    /// its position is `None` when it could not be mapped.
    pub diagnostics: Vec<crate::Diagnostic>,
}
#[derive(Debug)]
pub enum Packet {
    Event(Event),
    Frame {
        generation: u64,
        micros: u64,
        pixels: Vec<u32>,
    },
}
fn write_raw(w: &mut impl Write, kind: u8, data: &[u8]) -> io::Result<()> {
    if data.len() > MAX_PACKET {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "session packet too large",
        ));
    }
    w.write_all(&(data.len() as u32).to_le_bytes())?;
    w.write_all(&[kind])?;
    w.write_all(data)?;
    w.flush()
}
pub fn write_event(w: &mut impl Write, event: &Event) -> io::Result<()> {
    write_raw(w, 1, &serde_json::to_vec(event)?)
}
pub fn write_frame(
    w: &mut impl Write,
    generation: u64,
    micros: u64,
    pixels: &[u32],
) -> io::Result<()> {
    if pixels.len() != FRAME_PIXELS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid LCD size",
        ));
    }
    let payload_len = (16 + FRAME_PIXELS * 4) as u32;
    w.write_all(&payload_len.to_le_bytes())?;
    w.write_all(&[2u8])?;
    w.write_all(&generation.to_le_bytes())?;
    w.write_all(&micros.to_le_bytes())?;
    #[cfg(target_endian = "little")]
    {
        // SAFETY: u32 has 4 bytes, &[u32] can be viewed as &[u8] on little-endian targets.
        let bytes =
            unsafe { std::slice::from_raw_parts(pixels.as_ptr() as *const u8, FRAME_PIXELS * 4) };
        w.write_all(bytes)?;
    }
    #[cfg(not(target_endian = "little"))]
    {
        for pixel in pixels {
            w.write_all(&pixel.to_le_bytes())?;
        }
    }
    w.flush()
}
pub fn read_packet(r: &mut impl Read) -> io::Result<Packet> {
    let mut header = [0u8; 5];
    r.read_exact(&mut header)?;
    let len = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
    if len > MAX_PACKET {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "session packet exceeds limit",
        ));
    }
    let mut data = vec![0u8; len];
    r.read_exact(&mut data)?;
    match header[4] {
        1 => Ok(Packet::Event(serde_json::from_slice(&data)?)),
        2 if len == 16 + FRAME_PIXELS * 4 => Ok(Packet::Frame {
            generation: u64::from_le_bytes(data[..8].try_into().unwrap()),
            micros: u64::from_le_bytes(data[8..16].try_into().unwrap()),
            pixels: data[16..]
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                .collect(),
        }),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid session packet",
        )),
    }
}
pub fn write_command(w: &mut impl Write, c: &Command) -> io::Result<()> {
    let bytes = serde_json::to_vec(c)?;
    write_raw(w, 3, &bytes)
}
pub fn read_command(r: &mut impl Read) -> io::Result<Command> {
    let mut header = [0u8; 5];
    r.read_exact(&mut header)?;
    let len = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
    if header[4] != 3 || len > MAX_PACKET {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid command packet",
        ));
    }
    let mut bytes = vec![0; len];
    r.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_preserve_source_pixels() {
        let pixels = (0..FRAME_PIXELS as u32).collect::<Vec<_>>();
        let mut out = vec![];
        write_frame(&mut out, 7, 123, &pixels).unwrap();
        match read_packet(&mut out.as_slice()).unwrap() {
            Packet::Frame {
                generation,
                micros,
                pixels: p,
            } => {
                assert_eq!(p, pixels);
                assert_eq!((generation, micros), (7, 123));
            }
            _ => panic!(),
        }
    }
    #[test]
    fn malformed_and_truncated_packets_are_rejected() {
        for bytes in [
            vec![255, 255, 255, 255, 2],
            vec![0, 0, 0, 0, 2],
            vec![1, 0, 0, 0, 1],
        ] {
            assert!(read_packet(&mut bytes.as_slice()).is_err());
        }
    }
    #[test]
    fn unicode_source_round_trips() {
        let mut bytes = vec![];
        write_command(
            &mut bytes,
            &Command::Compile {
                request: 3,
                source: "LOCAL θ:=π;".into(),
            },
        )
        .unwrap();
        assert!(
            matches!(read_command(&mut bytes.as_slice()).unwrap(),Command::Compile{request:3,source} if source=="LOCAL θ:=π;")
        );
    }
    #[test]
    fn results_without_a_verdict_keep_the_original_wire_form() {
        let plain = Event::Result {
            request: 1,
            success: false,
            message: "busy".into(),
            check: None,
        };
        let json = serde_json::to_string(&plain).unwrap();
        assert!(!json.contains("check"), "{json}");
        // A peer that predates `check` is read as "no verdict".
        let old: Event = serde_json::from_str(
            r#"{"event":"result","request":2,"success":false,"message":"Error: Syntax Error"}"#,
        )
        .unwrap();
        assert!(matches!(old, Event::Result { check: None, .. }));
    }
    #[test]
    fn lifecycle_unsupported_and_import_round_trip() {
        for event in [
            Event::Lifecycle {
                state: Lifecycle::Ready,
            },
            Event::Unsupported {
                id: 0x10123,
                name: Some("PowerOff".into()),
                caller_pc: 0x3000_0000,
            },
        ] {
            let mut out = vec![];
            write_event(&mut out, &event).unwrap();
            let Packet::Event(back) = read_packet(&mut out.as_slice()).unwrap() else {
                panic!()
            };
            assert_eq!(format!("{back:?}"), format!("{event:?}"));
        }
        let mut bytes = vec![];
        write_command(
            &mut bytes,
            &Command::Import {
                request: 9,
                name: "M12HELLO".into(),
                source: "EXPORT M12HELLO() BEGIN END;".into(),
            },
        )
        .unwrap();
        assert!(matches!(
            read_command(&mut bytes.as_slice()).unwrap(),
            Command::Import { request: 9, name, .. } if name == "M12HELLO"
        ));
    }
    #[test]
    fn load_source_and_frame_request_round_trip() {
        let mut bytes = vec![];
        write_command(
            &mut bytes,
            &Command::LoadSource {
                request: 42,
                source: "EXPORT FOO() BEGIN END;".into(),
            },
        )
        .unwrap();
        assert!(matches!(
            read_command(&mut bytes.as_slice()).unwrap(),
            Command::LoadSource {
                request: 42,
                source
            } if source == "EXPORT FOO() BEGIN END;"
        ));

        bytes.clear();
        write_command(&mut bytes, &Command::FrameRequest).unwrap();
        assert!(matches!(
            read_command(&mut bytes.as_slice()).unwrap(),
            Command::FrameRequest
        ));
    }
}
