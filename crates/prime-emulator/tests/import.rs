//! M5-B importer: validation, transactional write and conflict handling
//! on temporary data directories (no firmware needed).

use std::path::{Path, PathBuf};

use prime_emulator::hpprgm;
use prime_emulator::import::{
    self, ImportError, ImportRequest, MAX_NAME_CHARS, TEMP_PREFIX, decode_source, validate_name,
};

const HELLO: &str = "EXPORT HELLO()\nBEGIN\n  PRINT(\"Hello from HP Prime!\");\nEND;\n";

/// A data directory as a firmware boot leaves it (`C/DATA` with files).
fn data_dir() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let data = d.path().join("C/DATA");
    std::fs::create_dir_all(data.join("&Function.hpappdir")).unwrap();
    std::fs::write(data.join("calc.hpsettings"), b"settings").unwrap();
    std::fs::write(data.join("&Function.hpappdir/&Function.hpapp"), b"app").unwrap();
    std::fs::create_dir_all(d.path().join("A")).unwrap();
    d
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

fn req(program: PathBuf, data: &Path) -> ImportRequest {
    ImportRequest {
        program,
        data_dir: data.to_path_buf(),
        name: None,
        replace: false,
        as_source: false,
    }
}

fn programs(data: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(data.join("C/DATA"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.to_ascii_lowercase().ends_with(".hpprgm") || n.starts_with(TEMP_PREFIX))
        .collect();
    v.sort();
    v
}

#[test]
fn imports_source_as_a_firmware_layout_container() {
    let d = data_dir();
    let src = tempfile::tempdir().unwrap();
    let p = write(src.path(), "HELLO.ppl", HELLO.as_bytes());
    let r = import::import(&req(p, d.path())).unwrap();
    assert!(r.imported && r.other_files_unchanged);
    assert_eq!(r.name, "HELLO");
    assert_eq!(r.guest_path, "C:\\DATA\\HELLO.hpprgm");
    assert_eq!(r.input.kind, "ppl-source");
    assert_eq!(r.input.encoding, Some("utf-8"));
    assert_eq!(r.input.line_endings, Some("lf"));
    assert_eq!(r.source_lines, 4);
    let bytes = std::fs::read(d.path().join("C/DATA/HELLO.hpprgm")).unwrap();
    assert_eq!(bytes, hpprgm::build_program(HELLO).unwrap().to_bytes());
    assert_eq!(r.sha256, prime_emulator::sha256::hex(&bytes));
    assert_eq!(
        programs(d.path()),
        vec!["HELLO.hpprgm"],
        "no temp file left"
    );
    // Other files untouched.
    assert_eq!(
        std::fs::read(d.path().join("C/DATA/calc.hpsettings")).unwrap(),
        b"settings"
    );
}

#[test]
fn line_endings_and_encodings_are_normalised_and_reported() {
    let lf = "A\nB\n";
    let cases: Vec<(Vec<u8>, &str, &str)> = vec![
        (b"A\r\nB\r\n".to_vec(), "utf-8", "crlf->lf"),
        (b"A\rB\r".to_vec(), "utf-8", "cr->lf"),
        (b"A\r\nB\n".to_vec(), "utf-8", "mixed->lf"),
        (
            [&[0xEF, 0xBB, 0xBF][..], b"A\nB\n"].concat(),
            "utf-8-bom",
            "lf",
        ),
        (
            [0xFF, 0xFE]
                .into_iter()
                .chain(lf.encode_utf16().flat_map(u16::to_le_bytes))
                .collect(),
            "utf-16le-bom",
            "lf",
        ),
        (
            [0xFE, 0xFF]
                .into_iter()
                .chain("A\r\nB\r\n".encode_utf16().flat_map(u16::to_be_bytes))
                .collect(),
            "utf-16be-bom",
            "crlf->lf",
        ),
    ];
    for (bytes, enc, le) in cases {
        let s = decode_source(&bytes).unwrap();
        assert_eq!((s.text.as_str(), s.encoding, s.line_endings), (lf, enc, le));
    }
    assert!(decode_source(&[0xC3, 0x28]).is_err(), "invalid UTF-8");
    assert!(decode_source(&[0xFF, 0xFE, 0x41]).is_err(), "odd UTF-16");
    assert!(decode_source(b"A\0B").is_err(), "NUL");
    assert!(decode_source(b"  \n").is_err(), "empty");
}

#[test]
fn name_rules() {
    for ok in ["HELLO", "a", "Test_2", &"A".repeat(MAX_NAME_CHARS)] {
        validate_name(ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
    // Firmware-verified rejections: leading digit, 32 characters.
    for bad in [
        "",
        "1A",
        "_X",
        "HELLO WORLD",
        "A-B",
        "É",
        "A.B",
        &"A".repeat(MAX_NAME_CHARS + 1),
    ] {
        assert!(
            matches!(validate_name(bad), Err(ImportError::InvalidName(_))),
            "{bad}"
        );
    }
}

#[test]
fn invalid_names_write_nothing() {
    let d = data_dir();
    let src = tempfile::tempdir().unwrap();
    let p = write(src.path(), "1BAD.ppl", HELLO.as_bytes());
    let e = import::import(&req(p.clone(), d.path())).unwrap_err();
    assert!(matches!(e, ImportError::InvalidName(_)));
    let mut r = req(p, d.path());
    r.name = Some("BAD NAME".into());
    assert!(matches!(
        import::import(&r),
        Err(ImportError::InvalidName(_))
    ));
    assert!(programs(d.path()).is_empty());
}

#[test]
fn duplicates_are_refused_case_insensitively_unless_replace() {
    let d = data_dir();
    let src = tempfile::tempdir().unwrap();
    let p = write(src.path(), "HELLO.ppl", HELLO.as_bytes());
    import::import(&req(p, d.path())).unwrap();
    let first = std::fs::read(d.path().join("C/DATA/HELLO.hpprgm")).unwrap();

    let v2 = write(
        src.path(),
        "hello.ppl",
        b"EXPORT HELLO() BEGIN RETURN 2; END;\n",
    );
    let e = import::import(&req(v2.clone(), d.path())).unwrap_err();
    assert!(matches!(e, ImportError::Exists { .. }), "{e}");
    assert_eq!(
        std::fs::read(d.path().join("C/DATA/HELLO.hpprgm")).unwrap(),
        first
    );

    // --replace: the new file takes the requested spelling, the old one is
    // gone, and the report names what was replaced.
    let mut r = req(v2, d.path());
    r.replace = true;
    let rep = import::import(&r).unwrap();
    let (old_path, old_sha) = rep.replaced.unwrap();
    assert!(old_path.ends_with("HELLO.hpprgm"));
    assert_eq!(old_sha, prime_emulator::sha256::hex(&first));
    assert!(rep.other_files_unchanged);
    assert_eq!(programs(d.path()), vec!["hello.hpprgm"]);
    let now = std::fs::read(d.path().join("C/DATA/hello.hpprgm")).unwrap();
    assert_eq!(
        hpprgm::parse(&now).unwrap().source().unwrap().source,
        "EXPORT HELLO() BEGIN RETURN 2; END;\n"
    );
}

#[test]
fn several_programs_coexist() {
    let d = data_dir();
    let src = tempfile::tempdir().unwrap();
    let progs = [
        ("HELLO", HELLO),
        ("SQUARE", "EXPORT SQUARE(N) BEGIN RETURN N*N; END;\n"),
        (
            "TEST",
            "EXPORT TEST() BEGIN LOCAL A; A := 42; PRINT(A); END;\n",
        ),
    ];
    for (name, text) in progs {
        let p = write(src.path(), &format!("{name}.ppl"), text.as_bytes());
        import::import(&req(p, d.path())).unwrap();
    }
    assert_eq!(
        import::list_programs(d.path()),
        vec!["HELLO", "SQUARE", "TEST"]
    );
    for (name, text) in progs {
        let b = std::fs::read(d.path().join(format!("C/DATA/{name}.hpprgm"))).unwrap();
        assert_eq!(hpprgm::parse(&b).unwrap().source().unwrap().source, text);
    }
}

#[test]
fn requires_a_firmware_initialised_data_directory() {
    let d = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let p = write(src.path(), "HELLO.ppl", HELLO.as_bytes());
    let e = import::import(&req(p, d.path())).unwrap_err();
    assert!(matches!(e, ImportError::NotInitialized(_)));
    assert!(!d.path().join("C").exists(), "nothing created");
}

#[test]
fn plain_text_named_hpprgm_is_not_a_container() {
    let d = data_dir();
    let src = tempfile::tempdir().unwrap();
    let p = write(src.path(), "HELLO.hpprgm", HELLO.as_bytes());
    let e = import::import(&req(p.clone(), d.path())).unwrap_err();
    assert!(matches!(e, ImportError::PlainTextContainer(_)), "{e}");
    assert!(programs(d.path()).is_empty());
    // Explicitly as source: imported and wrapped in a real container.
    let mut r = req(p, d.path());
    r.as_source = true;
    let rep = import::import(&r).unwrap();
    assert_eq!(rep.input.kind, "ppl-source");
    let b = std::fs::read(d.path().join("C/DATA/HELLO.hpprgm")).unwrap();
    assert_eq!(&b[..4], &hpprgm::MAGIC.to_le_bytes());
}

#[test]
fn genuine_containers_are_validated_and_copied_unchanged() {
    let d = data_dir();
    let src = tempfile::tempdir().unwrap();
    let container = hpprgm::build_program("EXPORT X() BEGIN END;")
        .unwrap()
        .to_bytes();
    let p = write(src.path(), "XPROG.hpprgm", &container);
    let rep = import::import(&req(p, d.path())).unwrap();
    assert_eq!(rep.input.kind, "hpprgm-container");
    assert_eq!(rep.name, "XPROG");
    assert_eq!(
        std::fs::read(d.path().join("C/DATA/XPROG.hpprgm")).unwrap(),
        container
    );

    // A corrupt container (magic, truncated) is refused, nothing written.
    let p = write(
        src.path(),
        "BROKEN.hpprgm",
        &container[..container.len() - 7],
    );
    let e = import::import(&req(p, d.path())).unwrap_err();
    assert!(matches!(e, ImportError::Container(_)), "{e}");
    assert_eq!(programs(d.path()), vec!["XPROG.hpprgm"]);
}

#[test]
fn stale_temp_files_are_removed_and_reported() {
    let d = data_dir();
    let stale = format!("{TEMP_PREFIX}OLD-1.tmp");
    std::fs::write(d.path().join("C/DATA").join(&stale), b"half").unwrap();
    let src = tempfile::tempdir().unwrap();
    let p = write(src.path(), "HELLO.ppl", HELLO.as_bytes());
    let rep = import::import(&req(p, d.path())).unwrap();
    assert_eq!(rep.removed_stale_temps, vec![stale]);
    assert_eq!(programs(d.path()), vec!["HELLO.hpprgm"]);
    // Temp names can never match the firmware's `*.hpprgm` enumeration.
    assert!(!TEMP_PREFIX.ends_with(".hpprgm"));
}

#[cfg(unix)]
#[test]
fn a_failed_write_leaves_no_partial_program() {
    use std::os::unix::fs::PermissionsExt;
    let d = data_dir();
    let dir = d.path().join("C/DATA");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let src = tempfile::tempdir().unwrap();
    let p = write(src.path(), "HELLO.ppl", HELLO.as_bytes());
    let r = import::import(&req(p, d.path()));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    if nix_is_root() {
        return; // root ignores the read-only mode
    }
    assert!(matches!(r, Err(ImportError::Io(_))), "{r:?}");
    assert!(programs(d.path()).is_empty(), "no program, no temp file");
}

#[cfg(unix)]
fn nix_is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .map(|l| l.split_whitespace().nth(1) == Some("0"))
        })
        .unwrap_or(false)
}
