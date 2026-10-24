//! `.hpprgm` container: layout, strict parsing and serialization (no
//! firmware needed).
//!
//! The expected bytes are assembled here by hand from the layout the M5
//! firmware runs established (the emulated 2.4.2 firmware's own save of
//! `HELLO` after leaving its Program Editor, minus the export cache it
//! adds): an independent statement of the format, not the serializer
//! checking itself.

use prime_emulator::hpprgm::{self, Body, Error, Object};

const TEMPLATE: &str = "EXPORT HELLO()\nBEGIN\n\nEND;\n";

fn le(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn name64(s: &str) -> Vec<u8> {
    let mut v: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    v.resize(64, 0);
    v
}

/// The firmware's structure for a program it has not checked yet.
fn expected_bytes(source: &str) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(le(0xB28A_617C));
    b.extend(le(0xFFFF_FFFE));
    b.extend(le(0));
    for ty in [0x007F_FF05, 0x023F_FF05, 0x00BF_FF05] {
        b.extend(le(8));
        b.extend(le(ty));
        b.extend(le(0));
    }
    let src = utf16z(source);
    let group_len = (4 + 4 + 64) + (4 + 4 + 4) + (4 + 4 + src.len());
    b.extend(le((4 + 4 + group_len) as u32));
    b.extend(le(0x0140_00BE));
    b.extend(le(group_len as u32));
    b.extend(le(0x44));
    b.extend(le(0x0040_008B));
    b.extend(name64("Main"));
    b.extend(le(8));
    b.extend(le(0x0080_0085));
    b.extend(le(0));
    b.extend(le((4 + src.len()) as u32));
    b.extend(le(0x00C0_009B));
    b.extend(&src);
    b.extend(le(0x39C));
    b.extend(le(0x0240_028E));
    for _ in 0..10 {
        b.extend(le(0x58));
        b.extend(le(0x44));
        b.extend(le(0x0040_028B));
        b.extend([0u8; 64]);
        b.extend(le(0x0C));
        b.extend(le(0x0080_0284));
        b.extend([0xFFu8; 8]);
    }
    b
}

#[test]
fn build_matches_the_firmware_layout_byte_for_byte() {
    let bytes = hpprgm::build_program(TEMPLATE).unwrap().to_bytes();
    assert_eq!(bytes, expected_bytes(TEMPLATE));
    // 1136 bytes: the firmware's 1232-byte save minus its 96-byte export
    // cache (one `HELLO` group).
    assert_eq!(bytes.len(), 1136);
}

#[test]
fn source_strings_are_not_padded() {
    // 40 characters + NUL = 82 bytes: the firmware stores it unaligned.
    let src = "EXPORT SQUARE(N) BEGIN RETURN N*N; END;\n";
    let bytes = hpprgm::build_program(src).unwrap().to_bytes();
    assert_eq!(bytes, expected_bytes(src));
    let c = hpprgm::parse(&bytes).unwrap();
    let files = c.section(hpprgm::SECTION_FILES).unwrap();
    assert_eq!(files.groups()[0][2].payload().unwrap().len(), 82);
    // The table follows at an offset that is not a multiple of 4 (0x14A in
    // the firmware's save, which also holds a 0x60-byte export cache).
    assert_eq!(
        c.section(hpprgm::SECTION_TABLE).unwrap().offset,
        0x14A - 0x60
    );
}

#[test]
fn parse_and_serialize_round_trip() {
    for src in [
        TEMPLATE,
        "EXPORT T() BEGIN PRINT(\"≥ → é Σ\"); END;",
        "// non-BMP: \u{1F600}\nEXPORT U() BEGIN END;",
    ] {
        let bytes = hpprgm::build_program(src).unwrap().to_bytes();
        let c = hpprgm::parse(&bytes).unwrap();
        assert_eq!(c.to_bytes(), bytes);
        let s = c.source().unwrap();
        assert_eq!(s.source, src);
        assert_eq!(s.name, "Main");
        assert!(s.has_text);
        assert_eq!(c.header_ints(), [0, 0, 0]);
        assert!(c.exports().is_empty());
    }
}

/// A container with an export cache like the firmware writes after a
/// check: round-trips exactly and reports the cache.
#[test]
fn export_cache_is_read_and_preserved() {
    let mut c = hpprgm::build_program(TEMPLATE).unwrap();
    c.objects.insert(
        3,
        Object::list(
            hpprgm::SECTION_EXPORTS,
            vec![vec![
                Object::data(0x0040_020B, name64("HELLO")),
                Object::data(0x0080_0205, 9u32.to_le_bytes().to_vec()),
            ]],
        ),
    );
    let bytes = c.to_bytes();
    assert_eq!(bytes.len(), 1232, "the firmware's saved size");
    let back = hpprgm::parse(&bytes).unwrap();
    assert_eq!(back.exports(), vec![("HELLO".to_owned(), 9)]);
    assert_eq!(back.to_bytes(), bytes);
    assert_eq!(back.source().unwrap().source, TEMPLATE);
}

#[test]
fn a_created_but_unedited_program_has_empty_source() {
    // The catalog's New writes `(name64, int)` without a string object.
    let mut c = hpprgm::build_program(TEMPLATE).unwrap();
    let files = c
        .objects
        .iter_mut()
        .find(|o| o.ty == hpprgm::SECTION_FILES)
        .unwrap();
    if let Body::List(groups) = &mut files.body {
        groups[0].truncate(2);
    }
    let bytes = c.to_bytes();
    let s = hpprgm::parse(&bytes).unwrap().source().unwrap();
    assert!(!s.has_text);
    assert_eq!(s.source, "");
}

#[test]
fn name64_padding_is_not_significant() {
    let mut bytes = hpprgm::build_program(TEMPLATE).unwrap().to_bytes();
    let clean = bytes.clone();
    // Garbage after "Main\0" (as uninitialised memory in an emulated save).
    let main_at = bytes
        .windows(10)
        .position(|w| w == utf16z("Main").as_slice())
        .unwrap();
    bytes[main_at + 10..main_at + 64].fill(0xA5);
    let c = hpprgm::parse(&bytes).unwrap();
    assert_eq!(c.source().unwrap().name, "Main");
    assert_eq!(hpprgm::canonical_padding(&bytes).unwrap(), clean);
}

#[test]
fn rejects_bad_magic_and_plain_text() {
    let text = b"EXPORT HELLO()\nBEGIN\nEND;\n";
    assert!(matches!(hpprgm::parse(text), Err(Error::BadMagic(_))));
    // UTF-16 text with a BOM is not a container either.
    let mut u16text = vec![0xFF, 0xFE];
    u16text.extend(utf16z("EXPORT X()"));
    assert!(matches!(hpprgm::parse(&u16text), Err(Error::BadMagic(_))));
    assert!(matches!(
        hpprgm::parse(&[0x7C, 0x61]),
        Err(Error::Truncated { .. })
    ));
}

#[test]
fn rejects_bad_preamble_and_header_ints() {
    let good = hpprgm::build_program(TEMPLATE).unwrap().to_bytes();
    let mut b = good.clone();
    b[4] = 0;
    assert!(matches!(
        hpprgm::parse(&b),
        Err(Error::BadHeader { offset: 4, .. })
    ));
    let mut b = good.clone();
    b[0x10] = 0x06; // first header int's type
    assert!(matches!(hpprgm::parse(&b), Err(Error::BadHeader { .. })));
}

#[test]
fn rejects_truncation_and_bad_lengths() {
    let good = hpprgm::build_program(TEMPLATE).unwrap().to_bytes();
    for cut in [good.len() - 1, good.len() - 5, 0x31, 13] {
        let r = hpprgm::parse(&good[..cut]);
        assert!(
            matches!(
                r,
                Err(Error::Truncated { .. } | Error::BadLength { .. } | Error::GroupOverrun { .. })
            ),
            "cut {cut}: {r:?}"
        );
    }
    // An object length past the end of the file.
    let mut b = good.clone();
    b[0x30..0x34].copy_from_slice(&le(0xFFFF));
    assert!(matches!(
        hpprgm::parse(&b),
        Err(Error::BadLength { offset: 0x30, .. })
    ));
    // A group length past the end of its list.
    let mut b = good.clone();
    b[0x38..0x3C].copy_from_slice(&le(0x400));
    assert!(matches!(
        hpprgm::parse(&b),
        Err(Error::GroupOverrun { offset: 0x38 })
    ));
    // Trailing garbage after the last object.
    let mut b = good;
    b.extend([1, 2, 3]);
    assert!(hpprgm::parse(&b).is_err());
}

fn with_source_payload(payload: Vec<u8>) -> Vec<u8> {
    let mut c = hpprgm::build_program("X").unwrap();
    let files = c
        .objects
        .iter_mut()
        .find(|o| o.ty == hpprgm::SECTION_FILES)
        .unwrap();
    if let Body::List(groups) = &mut files.body {
        groups[0][2] = Object::data(0x00C0_009B, payload);
    }
    c.to_bytes()
}

#[test]
fn rejects_bad_source_strings() {
    let source = |b: Vec<u8>| hpprgm::parse(&b).unwrap().source();
    // No NUL terminator.
    let r = source(with_source_payload(vec![b'A', 0, b'B', 0]));
    assert!(matches!(r, Err(Error::MissingNul { .. })), "{r:?}");
    // Odd byte count.
    let r = source(with_source_payload(vec![b'A', 0, 0]));
    assert!(matches!(r, Err(Error::InvalidUtf16 { .. })), "{r:?}");
    // Lone surrogate.
    let r = source(with_source_payload(vec![0x00, 0xD8, 0, 0]));
    assert!(matches!(r, Err(Error::InvalidUtf16 { .. })), "{r:?}");
    // Data after the terminator.
    let r = source(with_source_payload(vec![b'A', 0, 0, 0, b'B', 0]));
    assert!(matches!(r, Err(Error::BadSource { .. })), "{r:?}");
}

#[test]
fn rejects_missing_or_duplicate_source_sections() {
    let mut c = hpprgm::build_program(TEMPLATE).unwrap();
    let files = c
        .objects
        .iter()
        .position(|o| o.ty == hpprgm::SECTION_FILES)
        .unwrap();
    let section = c.objects[files].clone();
    c.objects.push(section);
    let r = hpprgm::parse(&c.to_bytes()).unwrap().source();
    assert_eq!(r, Err(Error::DuplicateSource));
    c.objects.retain(|o| o.ty != hpprgm::SECTION_FILES);
    let r = hpprgm::parse(&c.to_bytes()).unwrap().source();
    assert_eq!(r, Err(Error::MissingSource));
}

#[test]
fn build_refuses_text_the_firmware_cannot_store() {
    assert_eq!(
        hpprgm::build_program("A\0B").unwrap_err(),
        hpprgm::BuildError::SourceContainsNul
    );
    assert_eq!(
        hpprgm::build_program("A\r\nB").unwrap_err(),
        hpprgm::BuildError::SourceContainsCr
    );
}
