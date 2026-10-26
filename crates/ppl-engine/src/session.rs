//! Versioned local calculator-session transport. No GUI, USB, or Unicorn dependency.
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
pub const VERSION: u32 = 1;
pub const MAX_PACKET: usize = 4 * 1024 * 1024;
pub const FRAME_PIXELS: usize = 320 * 240;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
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
    Run {
        request: u64,
        source: String,
        entry: Option<String>,
        arguments: String,
    },
    Stop,
    Shutdown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Hello {
        version: u32,
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
    },
    Error {
        message: String,
    },
    Closed {
        message: String,
    },
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
