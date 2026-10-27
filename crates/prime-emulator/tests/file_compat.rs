//! Permanent authored file corpus. Paths are anchored to the manifest, never cwd.
#[path = "support/hpprgm_corpus.rs"]
mod corpus;
use ppl_engine::source::{Source, decode_program};
use prime_emulator::{hpprgm, import, sha256, storage};

#[test]
fn corpus_preserves_source_or_rejects_without_writes() {
    let cases = corpus::cases();
    assert!(cases.len() >= 32);
    assert!(cases.iter().filter(|c| c.firmware).count() >= 20);
    for c in cases {
        let path = corpus::root().join(&c.path);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(sha256::hex(&bytes), c.sha256, "{} hash", c.id);
        assert_eq!(bytes.len() as u64, c.size, "{} size", c.id);
        let decoded = decode_program(&bytes);
        let loaded = Source::file(&path);
        if let Some(raw) = &c.source {
            let d = decoded.unwrap_or_else(|e| panic!("{}: {e}", c.id));
            let loaded = loaded.unwrap();
            assert_eq!(&d.original_text, raw, "{} original", c.id);
            assert_eq!(d.encoding, c.encoding, "{} encoding", c.id);
            assert_eq!(d.container, c.container, "{} representation", c.id);
            assert_eq!(loaded.original_text, *raw, "{} engine source", c.id);
            assert_eq!(loaded.text, raw.replace("\r\n", "\n").replace('\r', "\n"));
            assert_eq!(loaded.bytes, bytes);
            assert!(loaded.identity.path.as_ref().unwrap().is_absolute());
            if c.container {
                let native = hpprgm::parse(&bytes).unwrap();
                assert_eq!(native.to_bytes(), bytes, "{} raw round trip", c.id);
                assert_eq!(native.source().unwrap().source, *raw);
            }
        } else {
            assert!(
                decoded.is_err() && loaded.is_err(),
                "{} must fail decoding",
                c.id
            );
            if bytes.starts_with(&hpprgm::MAGIC.to_le_bytes()) {
                assert!(
                    hpprgm::parse(&bytes).and_then(|c| c.source()).is_err(),
                    "{} parser disagreement",
                    c.id
                );
            }
        }
        let profile = tempfile::tempdir().unwrap();
        let data = profile.path().join("C/DATA");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("UNRELATED.hpprgm"), b"sentinel").unwrap();
        let before = storage::snapshot(profile.path());
        let result = import::import(&import::ImportRequest {
            program: path,
            data_dir: profile.path().into(),
            name: c.name,
            replace: false,
            as_source: c.as_source,
        });
        match c.import_result.as_str() {
            "ok" => {
                let r = result.unwrap_or_else(|e| panic!("{}: {e}", c.id));
                assert!(r.other_files_unchanged);
                let stored = std::fs::read(&r.host_path).unwrap();
                assert_eq!(sha256::hex(&stored), r.sha256);
                let src = hpprgm::parse(&stored).unwrap().source().unwrap().source;
                let expected = c.source.unwrap();
                assert_eq!(
                    src,
                    if c.container {
                        expected
                    } else {
                        expected.replace("\r\n", "\n").replace('\r', "\n")
                    },
                    "{} imported source",
                    c.id
                );
                if c.container {
                    assert_eq!(stored, bytes, "{} container copied exactly", c.id);
                }
            }
            expected => {
                let e = result.unwrap_err();
                let actual = match e {
                    import::ImportError::InvalidName(_) => "invalid-name",
                    import::ImportError::PlainTextContainer(_) => "plain-text",
                    import::ImportError::Container(_) => "container",
                    import::ImportError::Source(_) => "source",
                    _ => panic!("{} unexpected error: {e}", c.id),
                };
                assert_eq!(actual, expected, "{} rejection", c.id);
                assert_eq!(
                    storage::snapshot(profile.path()),
                    before,
                    "{} changed profile on failure",
                    c.id
                );
            }
        }
        assert_eq!(
            std::fs::read(data.join("UNRELATED.hpprgm")).unwrap(),
            b"sentinel"
        );
    }
}

#[test]
fn every_truncation_and_partial_group_header_is_a_safe_error() {
    let bytes = std::fs::read(corpus::root().join("files/MINIMAL.hpprgm")).unwrap();
    for n in 0..bytes.len() {
        // A prefix ending at an object boundary may be structurally valid;
        // source validation must still never panic or salvage partial text.
        let a = hpprgm::parse(&bytes[..n]).and_then(|c| c.source());
        let b = decode_program(&bytes[..n]);
        if n >= 4 {
            assert_eq!(a.is_ok(), b.is_ok(), "prefix {n}");
        }
    }
    for remaining in 1..4 {
        let mut bad = bytes[..48].to_vec();
        bad.extend_from_slice(&(4u32 + remaining).to_le_bytes());
        bad.extend_from_slice(&hpprgm::SECTION_FILES.to_le_bytes());
        bad.extend_from_slice(&[0; 8]);
        assert!(hpprgm::parse(&bad).is_err());
        assert!(decode_program(&bad).is_err());
    }
}

#[test]
fn single_byte_mutations_never_panic_or_change_accepted_bytes_on_reserialization() {
    let original = std::fs::read(corpus::root().join("files/MINIMAL.hpprgm")).unwrap();
    for at in 0..original.len() {
        for mask in [1, 0x80, 0xff] {
            let mut bytes = original.clone();
            bytes[at] ^= mask;
            if let Ok(c) = hpprgm::parse(&bytes) {
                assert_eq!(c.to_bytes(), bytes, "byte {at}, mask {mask}");
                // Decoding may reject source encodings but must never salvage
                // a structural error or remove text after an embedded NUL.
                let _ = c.source();
            }
            let _ = decode_program(&bytes);
        }
    }
}
