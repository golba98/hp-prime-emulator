//! Milestone 3 Challenger 1 Empirical Stress & Adversarial Protocol Test Suite
//! Authored by Challenger 1 for Milestone 3: Protocol Framing, Wire Transport & Monotonic Frame Sequence Stress Testing
//!
//! Mission Requirements:
//! 1. Command serialization & deserialization round-trip across all variants including `LoadSource` and `FrameRequest`.
//! 2. Binary frame wire format verification (5-byte header, little-endian payload, zero corruption across 320x240 pixels).
//! 3. Packet bounds checking (reject packets exceeding `MAX_PACKET = 4MB`).
//! 4. Monotonic generation sequence enforcement in display texture updates (drop out-of-order, drop stale, handle `u64::MAX` sentinel).

use ppl_engine::session::{self, Command, Event, FRAME_PIXELS, MAX_PACKET, Packet, VERSION};
use ppl_gui::ide::display::CalculatorDisplay;
use std::io::{Cursor, ErrorKind};
use std::sync::{Arc, Mutex};
use std::thread;

// ============================================================================
// Deterministic Pseudo-Random Generator for Reproducible Stress Testing
// ============================================================================

struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0xDEAD_BEEF_CAFE_BABE
            } else {
                seed
            },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }
}

// ============================================================================
// AREA 1: Command Serialization & Deserialization Round-Trip Across All Variants
// ============================================================================

#[test]
fn test_all_command_variants_round_trip() {
    let test_commands = vec![
        // 1. Key
        Command::Key {
            code: 0,
            down: true,
        },
        Command::Key {
            code: 50,
            down: false,
        },
        Command::Key {
            code: 191,
            down: true,
        },
        Command::Key {
            code: 0xE047,
            down: false,
        },
        Command::Key {
            code: u16::MAX,
            down: true,
        },
        // 2. Touch
        Command::Touch {
            status: 1,
            x: 0,
            y: 0,
        },
        Command::Touch {
            status: 2,
            x: 319,
            y: 239,
        },
        Command::Touch {
            status: 8,
            x: 160,
            y: 120,
        },
        Command::Touch {
            status: 99,
            x: u16::MAX,
            y: u16::MAX,
        },
        // 3. ReleaseAll
        Command::ReleaseAll,
        // 4. Script
        Command::Script {
            keys: String::new(),
        },
        Command::Script {
            keys: "ENTER UP DOWN LEFT RIGHT SIN COS TAN".into(),
        },
        Command::Script {
            keys: "1 2 + #FFh θ π Σ √ \n \t \"special\"".into(),
        },
        // 5. LoadSource
        Command::LoadSource {
            request: 0,
            source: String::new(),
        },
        Command::LoadSource {
            request: 42,
            source: "EXPORT FOO() BEGIN RETURN 1; END;".into(),
        },
        Command::LoadSource {
            request: u64::MAX,
            source: "LOCAL θ := π; LOCAL Σ := 0; // Unicode math\nRETURN √θ;".into(),
        },
        // 6. FrameRequest
        Command::FrameRequest,
        // 7. Compile
        Command::Compile {
            request: 101,
            source: "EXPORT MAIN()\nBEGIN\n  PRINT(\"Hello HP Prime\");\nEND;\n".into(),
        },
        Command::Compile {
            request: u64::MAX,
            source: String::new(),
        },
        // 8. Run
        Command::Run {
            request: 202,
            source: "EXPORT ADD(a, b) BEGIN RETURN a + b; END;".into(),
            entry: Some("ADD".into()),
            arguments: "10, 20".into(),
        },
        Command::Run {
            request: 203,
            source: "EXPORT MAIN() BEGIN END;".into(),
            entry: None,
            arguments: String::new(),
        },
        // 9. Stop
        Command::Stop,
        // 10. Shutdown
        Command::Shutdown,
    ];

    for (i, cmd) in test_commands.iter().enumerate() {
        let mut buffer = Vec::new();
        session::write_command(&mut buffer, cmd)
            .unwrap_or_else(|e| panic!("Failed to write command variant #{i}: {e:?}"));

        let mut cursor = Cursor::new(&buffer);
        let decoded = session::read_command(&mut cursor)
            .unwrap_or_else(|e| panic!("Failed to read command variant #{i}: {e:?}"));

        // Verify remaining cursor bytes is 0
        assert_eq!(
            cursor.position() as usize,
            buffer.len(),
            "Did not consume all bytes for command variant #{i}"
        );

        // Verify variant equality
        match (cmd, &decoded) {
            (Command::Key { code: c1, down: d1 }, Command::Key { code: c2, down: d2 }) => {
                assert_eq!((c1, d1), (c2, d2));
            }
            (
                Command::Touch {
                    status: s1,
                    x: x1,
                    y: y1,
                },
                Command::Touch {
                    status: s2,
                    x: x2,
                    y: y2,
                },
            ) => {
                assert_eq!((s1, x1, y1), (s2, x2, y2));
            }
            (Command::ReleaseAll, Command::ReleaseAll) => {}
            (Command::Script { keys: k1 }, Command::Script { keys: k2 }) => {
                assert_eq!(k1, k2);
            }
            (
                Command::LoadSource {
                    request: r1,
                    source: s1,
                },
                Command::LoadSource {
                    request: r2,
                    source: s2,
                },
            ) => {
                assert_eq!((r1, s1), (r2, s2));
            }
            (Command::FrameRequest, Command::FrameRequest) => {}
            (
                Command::Compile {
                    request: r1,
                    source: s1,
                },
                Command::Compile {
                    request: r2,
                    source: s2,
                },
            ) => {
                assert_eq!((r1, s1), (r2, s2));
            }
            (
                Command::Run {
                    request: r1,
                    source: s1,
                    entry: e1,
                    arguments: a1,
                },
                Command::Run {
                    request: r2,
                    source: s2,
                    entry: e2,
                    arguments: a2,
                },
            ) => {
                assert_eq!((r1, s1, e1, a1), (r2, s2, e2, a2));
            }
            (Command::Stop, Command::Stop) => {}
            (Command::Shutdown, Command::Shutdown) => {}
            _ => panic!("Command mismatch: sent {cmd:?}, got {decoded:?}"),
        }
    }
}

#[test]
fn test_all_event_variants_round_trip() {
    let test_events = [
        Event::Hello { version: VERSION },
        Event::Phase {
            phase: "compiling".into(),
        },
        Event::Phase {
            phase: "ready".into(),
        },
        Event::Status {
            message: "Calculator ready".into(),
        },
        Event::Result {
            request: 12345,
            success: true,
            message: "Compilation succeeded".into(),
            check: None,
        },
        Event::Result {
            request: 67890,
            success: false,
            message: "Syntax error on line 4".into(),
            check: None,
        },
        Event::Result {
            request: 67891,
            success: false,
            message: "Error: Syntax Error".into(),
            check: Some(session::CheckVerdict {
                accepted: false,
                source_sha256: "00".repeat(32),
                diagnostics: vec![ppl_engine::Diagnostic {
                    source: "guest-firmware".into(),
                    severity: "error".into(),
                    code: None,
                    message: Some("Error: Syntax Error".into()),
                    line: Some(3),
                    column: Some(9),
                    raw_position: Some(serde_json::json!({"line": 3, "offset_utf16": 30})),
                }],
            }),
        },
        Event::Error {
            message: "Fatal guest fault at 0x00010000".into(),
        },
        Event::Closed {
            message: "Local firmware session closed normally".into(),
        },
    ];

    for (i, ev) in test_events.iter().enumerate() {
        let mut buffer = Vec::new();
        session::write_event(&mut buffer, ev)
            .unwrap_or_else(|e| panic!("Failed to write event variant #{i}: {e:?}"));

        let mut cursor = Cursor::new(&buffer);
        let packet = session::read_packet(&mut cursor)
            .unwrap_or_else(|e| panic!("Failed to read event packet #{i}: {e:?}"));

        assert_eq!(cursor.position() as usize, buffer.len());

        match (ev, packet) {
            (Event::Hello { version: v1 }, Packet::Event(Event::Hello { version: v2 })) => {
                assert_eq!(v1, &v2);
            }
            (Event::Phase { phase: p1 }, Packet::Event(Event::Phase { phase: p2 })) => {
                assert_eq!(p1, &p2);
            }
            (Event::Status { message: m1 }, Packet::Event(Event::Status { message: m2 })) => {
                assert_eq!(m1, &m2);
            }
            (
                Event::Result {
                    request: r1,
                    success: s1,
                    message: m1,
                    check: c1,
                },
                Packet::Event(Event::Result {
                    request: r2,
                    success: s2,
                    message: m2,
                    check: c2,
                }),
            ) => {
                assert_eq!((r1, s1, m1, c1), (&r2, &s2, &m2, &c2));
            }
            (Event::Error { message: m1 }, Packet::Event(Event::Error { message: m2 })) => {
                assert_eq!(m1, &m2);
            }
            (Event::Closed { message: m1 }, Packet::Event(Event::Closed { message: m2 })) => {
                assert_eq!(m1, &m2);
            }
            _ => panic!("Event mismatch: expected {ev:?}"),
        }
    }
}

#[test]
fn test_interleaved_command_stream_fidelity() {
    let mut rng = XorShift64::new(0x1337_C0DE_CAFE_FEED);
    let mut original_commands = Vec::new();
    let mut stream = Vec::new();

    for i in 0..100 {
        let cmd = match i % 10 {
            0 => Command::Key {
                code: (rng.next_u32() % 200) as u16,
                down: (rng.next_u32() % 2) == 1,
            },
            1 => Command::Touch {
                status: match rng.next_u32() % 3 {
                    0 => 1,
                    1 => 2,
                    _ => 8,
                },
                x: (rng.next_u32() % 320) as u16,
                y: (rng.next_u32() % 240) as u16,
            },
            2 => Command::ReleaseAll,
            3 => Command::Script {
                keys: format!("STEP_{}", rng.next_u32()),
            },
            4 => Command::LoadSource {
                request: rng.next_u64(),
                source: format!(
                    "// Program revision {}\nEXPORT P_{}() BEGIN END;\n",
                    i,
                    rng.next_u32()
                ),
            },
            5 => Command::FrameRequest,
            6 => Command::Compile {
                request: rng.next_u64(),
                source: format!("EXPORT COMP_{}() BEGIN RETURN {}; END;", i, i * 10),
            },
            7 => Command::Run {
                request: rng.next_u64(),
                source: "EXPORT RUN() BEGIN RETURN 0; END;".into(),
                entry: if (rng.next_u32() % 2) == 0 {
                    Some("RUN".into())
                } else {
                    None
                },
                arguments: format!("{}", rng.next_u32()),
            },
            8 => Command::Stop,
            _ => Command::Shutdown,
        };

        session::write_command(&mut stream, &cmd).unwrap();
        original_commands.push(cmd);
    }

    let mut cursor = Cursor::new(&stream);
    for (i, expected) in original_commands.iter().enumerate() {
        let decoded = session::read_command(&mut cursor)
            .unwrap_or_else(|e| panic!("Stream read failed at index {i}: {e:?}"));

        match (expected, decoded) {
            (Command::Key { code: c1, down: d1 }, Command::Key { code: c2, down: d2 }) => {
                assert_eq!((c1, d1), (&c2, &d2), "Mismatch at {i}");
            }
            (
                Command::Touch {
                    status: s1,
                    x: x1,
                    y: y1,
                },
                Command::Touch {
                    status: s2,
                    x: x2,
                    y: y2,
                },
            ) => {
                assert_eq!((s1, x1, y1), (&s2, &x2, &y2), "Mismatch at {i}");
            }
            (Command::ReleaseAll, Command::ReleaseAll) => {}
            (Command::Script { keys: k1 }, Command::Script { keys: k2 }) => {
                assert_eq!(k1, &k2, "Mismatch at {i}");
            }
            (
                Command::LoadSource {
                    request: r1,
                    source: s1,
                },
                Command::LoadSource {
                    request: r2,
                    source: s2,
                },
            ) => {
                assert_eq!((r1, s1), (&r2, &s2), "Mismatch at {i}");
            }
            (Command::FrameRequest, Command::FrameRequest) => {}
            (
                Command::Compile {
                    request: r1,
                    source: s1,
                },
                Command::Compile {
                    request: r2,
                    source: s2,
                },
            ) => {
                assert_eq!((r1, s1), (&r2, &s2), "Mismatch at {i}");
            }
            (
                Command::Run {
                    request: r1,
                    source: s1,
                    entry: e1,
                    arguments: a1,
                },
                Command::Run {
                    request: r2,
                    source: s2,
                    entry: e2,
                    arguments: a2,
                },
            ) => {
                assert_eq!((r1, s1, e1, a1), (&r2, &s2, &e2, &a2), "Mismatch at {i}");
            }
            (Command::Stop, Command::Stop) => {}
            (Command::Shutdown, Command::Shutdown) => {}
            (exp, got) => panic!("Interleaved command mismatch at {i}: exp {exp:?}, got {got:?}"),
        }
    }

    assert_eq!(cursor.position() as usize, stream.len());
}

#[test]
fn test_large_source_payloads_round_trip() {
    // Test LoadSource and Compile with large source programs (100 KB, 500 KB, 1 MB, 3 MB)
    for size in [100 * 1024, 500 * 1024, 1024 * 1024, 3 * 1024 * 1024] {
        let header = "EXPORT BENCH()\nBEGIN\n";
        let footer = "\n  RETURN 42;\nEND;\n";
        let line = "  LOCAL x := 12345; x := x + 1;\n";
        let repeats = (size - header.len() - footer.len()) / line.len();
        let mut source = String::with_capacity(size);
        source.push_str(header);
        for _ in 0..repeats {
            source.push_str(line);
        }
        source.push_str(footer);

        let cmd = Command::LoadSource {
            request: 9999,
            source: source.clone(),
        };

        let mut buf = Vec::new();
        session::write_command(&mut buf, &cmd).unwrap();

        let mut cursor = Cursor::new(&buf);
        let decoded = session::read_command(&mut cursor).unwrap();

        match decoded {
            Command::LoadSource { request, source: s } => {
                assert_eq!(request, 9999);
                assert_eq!(s.len(), source.len());
                assert_eq!(s, source);
            }
            _ => panic!("Expected LoadSource"),
        }
    }
}

// ============================================================================
// AREA 2: Binary Frame Wire Format Verification & Zero Pixel Corruption
// ============================================================================

#[test]
fn test_frame_wire_header_format_exact_bytes() {
    let dummy_pixels = vec![0x00112233u32; FRAME_PIXELS];
    let generation: u64 = 0x0123_4567_89AB_CDEF;
    let micros: u64 = 0xFEDC_BA98_7654_3210;

    let mut wire = Vec::new();
    session::write_frame(&mut wire, generation, micros, &dummy_pixels).unwrap();

    // Total length: 5 byte header + 16 byte metadata + 307200 pixel bytes = 307221 bytes
    let expected_payload_len = (16 + FRAME_PIXELS * 4) as u32; // 307216
    assert_eq!(wire.len(), 5 + expected_payload_len as usize);

    // Byte 0..4: payload length in Little-Endian
    let wire_payload_len = u32::from_le_bytes(wire[0..4].try_into().unwrap());
    assert_eq!(wire_payload_len, expected_payload_len);
    assert_eq!(&wire[0..4], &expected_payload_len.to_le_bytes());

    // Byte 4: packet kind = 2 (Frame)
    assert_eq!(wire[4], 2u8);

    // Byte 5..13: generation in Little-Endian
    let wire_generation = u64::from_le_bytes(wire[5..13].try_into().unwrap());
    assert_eq!(wire_generation, generation);
    assert_eq!(&wire[5..13], &generation.to_le_bytes());

    // Byte 13..21: micros timestamp in Little-Endian
    let wire_micros = u64::from_le_bytes(wire[13..21].try_into().unwrap());
    assert_eq!(wire_micros, micros);
    assert_eq!(&wire[13..21], &micros.to_le_bytes());

    // Byte 21..25: first pixel 0x00112233 in Little-Endian
    assert_eq!(&wire[21..25], &0x00112233u32.to_le_bytes());
}

#[test]
fn test_zero_pixel_corruption_across_full_framebuffer() {
    let mut rng = XorShift64::new(0xCAFE_BABE_0123_4567);

    // 1. Random XRGB8888 pixels
    let mut random_pixels = Vec::with_capacity(FRAME_PIXELS);
    for _ in 0..FRAME_PIXELS {
        // High byte 0x00, 24-bit RGB
        random_pixels.push(rng.next_u32() & 0x00FF_FFFF);
    }

    let mut buf = Vec::new();
    session::write_frame(&mut buf, 100, 500_000, &random_pixels).unwrap();

    let packet = session::read_packet(&mut buf.as_slice()).unwrap();
    match packet {
        Packet::Frame {
            generation,
            micros,
            pixels,
        } => {
            assert_eq!(generation, 100);
            assert_eq!(micros, 500_000);
            assert_eq!(pixels.len(), FRAME_PIXELS);
            for (idx, (&expected, &actual)) in random_pixels.iter().zip(pixels.iter()).enumerate() {
                assert_eq!(
                    expected,
                    actual,
                    "Pixel corruption at linear index {idx} (x={}, y={}): expected 0x{expected:08X}, got 0x{actual:08X}",
                    idx % 320,
                    idx / 320
                );
            }
        }
        _ => panic!("Expected Packet::Frame"),
    }

    // 2. High-bit stress (0xFF000000, 0xAA000000, 0x55000000)
    let mut high_bit_pixels = Vec::with_capacity(FRAME_PIXELS);
    for i in 0..FRAME_PIXELS {
        let pattern = match i % 4 {
            0 => 0xFF00_0000 | (i as u32 & 0x00FF_FFFF),
            1 => 0x8000_0000 | (i as u32 & 0x00FF_FFFF),
            2 => 0x00AA_BBCC,
            _ => 0x0000_0000,
        };
        high_bit_pixels.push(pattern);
    }

    buf.clear();
    session::write_frame(&mut buf, u64::MAX, u64::MAX, &high_bit_pixels).unwrap();
    let packet = session::read_packet(&mut buf.as_slice()).unwrap();
    match packet {
        Packet::Frame {
            generation,
            micros,
            pixels,
        } => {
            assert_eq!(generation, u64::MAX);
            assert_eq!(micros, u64::MAX);
            assert_eq!(pixels, high_bit_pixels);
        }
        _ => panic!("Expected Packet::Frame"),
    }
}

#[test]
fn test_write_frame_dimension_bounds_enforcement() {
    let mut buf = Vec::new();

    // 1. Empty pixel slice
    let err = session::write_frame(&mut buf, 1, 1, &[]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);

    // 2. Short pixel slice (FRAME_PIXELS - 1)
    let short = vec![0u32; FRAME_PIXELS - 1];
    let err = session::write_frame(&mut buf, 1, 1, &short).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);

    // 3. Overflow pixel slice (FRAME_PIXELS + 1)
    let long = vec![0u32; FRAME_PIXELS + 1];
    let err = session::write_frame(&mut buf, 1, 1, &long).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

// ============================================================================
// AREA 3: Packet Bounds Checking (MAX_PACKET = 4MB) & Stream Resilience
// ============================================================================

#[test]
fn test_packet_bounds_max_limit_rejection() {
    // 1. Packet claiming length MAX_PACKET + 1 must be rejected by read_packet
    let oversized_len = (MAX_PACKET + 1) as u32;
    let mut header = Vec::new();
    header.extend_from_slice(&oversized_len.to_le_bytes());
    header.push(1u8); // Event
    let err = session::read_packet(&mut header.as_slice()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert!(err.to_string().contains("session packet exceeds limit"));

    // 2. Command claiming length MAX_PACKET + 1 must be rejected by read_command
    let mut cmd_header = Vec::new();
    cmd_header.extend_from_slice(&oversized_len.to_le_bytes());
    cmd_header.push(3u8); // Command
    let err = session::read_command(&mut cmd_header.as_slice()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert!(err.to_string().contains("invalid command packet"));

    // 3. Packet claiming u32::MAX (4GB) must be rejected immediately without OOM
    let huge_len = u32::MAX;
    let mut huge_header = Vec::new();
    huge_header.extend_from_slice(&huge_len.to_le_bytes());
    huge_header.push(1u8);
    let err = session::read_packet(&mut huge_header.as_slice()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);

    // 4. Command claiming 2GB must be rejected immediately
    let gig_len = 0x8000_0000u32;
    let mut gig_header = Vec::new();
    gig_header.extend_from_slice(&gig_len.to_le_bytes());
    gig_header.push(3u8);
    let err = session::read_command(&mut gig_header.as_slice()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
}

#[test]
fn test_frame_packet_payload_length_mismatch() {
    // If packet kind is 2 (Frame), but payload_len is NOT 16 + FRAME_PIXELS * 4,
    // read_packet must reject it with InvalidData.
    for invalid_payload_len in [
        0u32, 1u32, 16u32, 307200u32, 307215u32, 307217u32, 400000u32,
    ] {
        let mut raw = Vec::new();
        raw.extend_from_slice(&invalid_payload_len.to_le_bytes());
        raw.push(2u8); // Kind: Frame
        raw.resize(5 + invalid_payload_len as usize, 0u8);

        let err = session::read_packet(&mut raw.as_slice()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(err.to_string().contains("invalid session packet"));
    }
}

#[test]
fn test_corrupted_and_truncated_packet_rejection() {
    // 1. Truncated headers (0, 1, 2, 3, 4 bytes)
    for len in 0..5 {
        let buf = vec![0u8; len];
        let err = session::read_packet(&mut buf.as_slice()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);

        let err = session::read_command(&mut buf.as_slice()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
    }

    // 2. Truncated payload (header claims 100 bytes, stream has 50 bytes)
    let mut partial = Vec::new();
    partial.extend_from_slice(&100u32.to_le_bytes());
    partial.push(1u8);
    partial.resize(5 + 50, 0x41); // Only 50 payload bytes
    let err = session::read_packet(&mut partial.as_slice()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnexpectedEof);

    let mut partial_cmd = Vec::new();
    partial_cmd.extend_from_slice(&100u32.to_le_bytes());
    partial_cmd.push(3u8);
    partial_cmd.resize(5 + 50, 0x41);
    let err = session::read_command(&mut partial_cmd.as_slice()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnexpectedEof);

    // 3. Unknown packet kind in read_packet (e.g. 0, 3, 4, 255)
    for kind in [0u8, 3u8, 4u8, 255u8] {
        let mut raw = Vec::new();
        let payload = b"{}";
        raw.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        raw.push(kind);
        raw.extend_from_slice(payload);
        let err = session::read_packet(&mut raw.as_slice()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    // 4. Wrong packet kind in read_command (e.g. 1 or 2 instead of 3)
    for kind in [1u8, 2u8] {
        let mut raw = Vec::new();
        let payload = b"{}";
        raw.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        raw.push(kind);
        raw.extend_from_slice(payload);
        let err = session::read_command(&mut raw.as_slice()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    // 5. Incomplete JSON payload (ends abruptly) -> converts to UnexpectedEof or Other
    let mut raw_bad_json = Vec::new();
    let bad_json = b"{\"command\": \"load_source\", \"request\": ";
    raw_bad_json.extend_from_slice(&(bad_json.len() as u32).to_le_bytes());
    raw_bad_json.push(3u8);
    raw_bad_json.extend_from_slice(bad_json);
    let err = session::read_command(&mut raw_bad_json.as_slice()).unwrap_err();
    assert!(
        err.kind() == ErrorKind::UnexpectedEof || err.kind() == ErrorKind::Other,
        "Expected UnexpectedEof or Other for incomplete JSON, got {:?}",
        err.kind()
    );

    // 6. Corrupted syntax JSON (illegal characters)
    let mut raw_syntax_err = Vec::new();
    let syntax_err = b"{\"command\": @@@INVALID@@@}";
    raw_syntax_err.extend_from_slice(&(syntax_err.len() as u32).to_le_bytes());
    raw_syntax_err.push(3u8);
    raw_syntax_err.extend_from_slice(syntax_err);
    let err2 = session::read_command(&mut raw_syntax_err.as_slice()).unwrap_err();
    assert!(
        err2.kind() != ErrorKind::UnexpectedEof,
        "Syntax error should not be UnexpectedEof"
    );
}

// ============================================================================
// AREA 4: Monotonic Generation Sequence Enforcement (`update_xrgb`)
// ============================================================================

#[test]
fn test_display_update_xrgb_strictly_monotonic_enforcement() {
    let ctx = egui::Context::default();
    let mut display = CalculatorDisplay::new();

    let red = vec![0x00FF_0000u32; FRAME_PIXELS];
    let green = vec![0x0000_FF00u32; FRAME_PIXELS];
    let blue = vec![0x0000_00FFu32; FRAME_PIXELS];

    // 1. Initial frame: generation = 0 with texture == None must be accepted
    assert!(display.texture.is_none());
    assert_eq!(display.last_generation, 0);
    display.update_xrgb(&ctx, &red, 0);
    assert_eq!(display.last_generation, 0);
    assert!(display.texture.is_some());

    // 2. Duplicate generation 0 must be dropped (texture already exists)
    display.update_xrgb(&ctx, &green, 0);
    assert_eq!(display.last_generation, 0);

    // 3. Stale generation (e.g. earlier generation) must be dropped
    // Note: Since last_generation is 0, any frame >= 1 is newer
    display.update_xrgb(&ctx, &green, 10);
    assert_eq!(display.last_generation, 10);

    // Now test stale drops for generation 0..=10
    for stale_gen in [0, 1, 5, 9, 10] {
        display.update_xrgb(&ctx, &red, stale_gen);
        assert_eq!(
            display.last_generation, 10,
            "Stale generation {stale_gen} was not dropped!"
        );
    }

    // 4. Newer generation advances last_generation
    display.update_xrgb(&ctx, &blue, 11);
    assert_eq!(display.last_generation, 11);

    display.update_xrgb(&ctx, &red, 20);
    assert_eq!(display.last_generation, 20);

    // Duplicate 20 dropped
    display.update_xrgb(&ctx, &blue, 20);
    assert_eq!(display.last_generation, 20);
}

#[test]
fn test_display_update_xrgb_out_of_order_burst_stress() {
    let ctx = egui::Context::default();
    let mut display = CalculatorDisplay::new();
    let pixels = vec![0x0012_3456u32; FRAME_PIXELS];

    // Initial seed
    display.update_xrgb(&ctx, &pixels, 50);
    assert_eq!(display.last_generation, 50);

    // Send an out-of-order burst of generations
    let sequence = [
        45, 48, 52, 49, 51, 55, 53, 50, 56, 54, 60, 59, 58, 57, 65, 62, 61, 70, 68,
    ];

    let mut expected_current = 50;
    for &incoming_gen in &sequence {
        display.update_xrgb(&ctx, &pixels, incoming_gen);
        if incoming_gen > expected_current {
            expected_current = incoming_gen;
        }
        assert_eq!(
            display.last_generation, expected_current,
            "Failed at incoming generation {incoming_gen}: expected last_generation {expected_current}, got {}",
            display.last_generation
        );
    }

    assert_eq!(display.last_generation, 70);
}

#[test]
fn test_display_update_xrgb_u64_max_sentinel_resets() {
    let ctx = egui::Context::default();
    let mut display = CalculatorDisplay::new();
    let pixels = vec![0x00A5_5A5Au32; FRAME_PIXELS];

    display.update_xrgb(&ctx, &pixels, 100);
    assert_eq!(display.last_generation, 100);

    // Simulate backend switch: last_generation is set to u64::MAX sentinel
    display.last_generation = u64::MAX;

    // After sentinel reset, ANY generation (even 0 or 1) must be accepted
    display.update_xrgb(&ctx, &pixels, 1);
    assert_eq!(
        display.last_generation, 1,
        "Sentinel u64::MAX failed to allow lower generation 1"
    );

    // Normal monotonicity resumes
    display.update_xrgb(&ctx, &pixels, 0); // Stale (0 < 1), dropped
    assert_eq!(display.last_generation, 1);

    display.update_xrgb(&ctx, &pixels, 2); // Newer, accepted
    assert_eq!(display.last_generation, 2);

    // Test sentinel reset multiple times
    for reset_gen in [10, 5, 200, 1, 0] {
        display.last_generation = u64::MAX;
        display.update_xrgb(&ctx, &pixels, reset_gen);
        assert_eq!(display.last_generation, reset_gen);
    }
}

#[test]
fn test_display_update_xrgb_invalid_buffer_size_rejection() {
    let ctx = egui::Context::default();
    let mut display = CalculatorDisplay::new();
    let valid_pixels = vec![0x00FF_FFFFu32; FRAME_PIXELS];

    display.update_xrgb(&ctx, &valid_pixels, 10);
    assert_eq!(display.last_generation, 10);

    // Invalid lengths: 0, 1, FRAME_PIXELS - 1, FRAME_PIXELS + 1
    for bad_len in [
        0,
        1,
        100,
        FRAME_PIXELS - 1,
        FRAME_PIXELS + 1,
        FRAME_PIXELS * 2,
    ] {
        let bad_pixels = vec![0x0011_2233u32; bad_len];
        // Even with newer generation 20, invalid pixel buffer must be rejected immediately
        display.update_xrgb(&ctx, &bad_pixels, 20);
        assert_eq!(
            display.last_generation, 10,
            "Invalid buffer length {bad_len} was improperly accepted!"
        );
    }
}

#[test]
fn test_concurrent_reader_lock_monotonic_ordering() {
    // Stress-test the reader pattern used in ppl_gui::ide::firmware:
    // `if lock.as_ref().is_none_or(|f| generation > f.generation) { *lock = Some(Frame { generation, ... }); }`
    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    struct TestFrame {
        generation: u64,
        pixels: Arc<[u32]>,
    }

    let shared_frame = Arc::new(Mutex::new(None::<TestFrame>));
    let mut handles = Vec::new();

    // Spawn 8 worker threads pushing frames with scrambled generations 1..=500
    for thread_id in 0..8 {
        let frame_ref = shared_frame.clone();
        handles.push(thread::spawn(move || {
            let mut rng = XorShift64::new(0xFEED_FACE + thread_id as u64);
            let dummy = Arc::new([0u32; 16]);
            for _ in 0..100 {
                let worker_gen = (rng.next_u64() % 500) + 1;
                let mut lock = frame_ref.lock().unwrap();
                if lock.as_ref().is_none_or(|f| worker_gen > f.generation) {
                    *lock = Some(TestFrame {
                        generation: worker_gen,
                        pixels: dummy.clone(),
                    });
                }
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // Now push 501
    {
        let mut lock = shared_frame.lock().unwrap();
        let cur = lock.as_ref().map(|f| f.generation).unwrap_or(0);
        assert!((1..=500).contains(&cur));

        let gen_501 = 501;
        if lock.as_ref().is_none_or(|f| gen_501 > f.generation) {
            *lock = Some(TestFrame {
                generation: gen_501,
                pixels: Arc::new([0u32; 16]),
            });
        }
        assert_eq!(lock.as_ref().unwrap().generation, 501);

        // Try pushing 500 (stale)
        let gen_stale = 500;
        if lock.as_ref().is_none_or(|f| gen_stale > f.generation) {
            *lock = Some(TestFrame {
                generation: gen_stale,
                pixels: Arc::new([0u32; 16]),
            });
        }
        // Stale must be dropped
        assert_eq!(lock.as_ref().unwrap().generation, 501);
    }
}
