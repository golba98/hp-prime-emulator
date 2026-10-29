//! Application compatibility suite (tests/runtime). The schema test needs no
//! firmware; the execution test is skipped, with a log line, without it.
use prime_emulator::{appsuite, keys, sha256};
use std::path::{Path, PathBuf};

fn suite_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/runtime")
}

#[test]
fn cases_parse_and_pin_their_sources() {
    let cases = appsuite::load_cases(&suite_root().join("cases")).unwrap();
    assert!(cases.len() >= 15, "{} cases", cases.len());
    for (path, case) in &cases {
        let program = path.parent().unwrap().join(&case.program);
        let bytes = std::fs::read(&program).unwrap();
        assert_eq!(sha256::hex(&bytes), case.source_sha256, "{}", case.id);
        if let Some(k) = &case.runtime_keys {
            keys::parse_keys(k).unwrap();
        }
        assert_eq!(
            path.file_stem().unwrap().to_str().unwrap(),
            case.id,
            "case file name"
        );
        for c in &case.checkpoint {
            let is_scripted = case.runtime_keys.as_deref().is_some_and(|k| {
                k.split(',')
                    .any(|i| i.strip_prefix("SNAP:") == Some(&c.label) || i == c.label)
            });
            assert!(
                c.label == "return" || is_scripted,
                "{}: {}",
                case.id,
                c.label
            );
        }
    }
}

#[test]
fn firmware_application_suite_matches_local_references() {
    let Some(firmware) = std::env::var_os("PRIME_TEST_FIRMWARE")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
    else {
        eprintln!("SKIP app suite: PRIME_TEST_FIRMWARE unset");
        return;
    };
    let factory_a = PathBuf::from(
        std::env::var_os("PRIME_TEST_FACTORY_A").expect("app suite requires PRIME_TEST_FACTORY_A"),
    );
    let out_dir = std::env::var_os("PRIME_TEST_APPSUITE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| tempfile::tempdir().unwrap().keep());
    let cfg = appsuite::SuiteConfig {
        root: suite_root(),
        firmware,
        os_rom: std::env::var_os("PRIME_TEST_OS_ROM").map(PathBuf::from),
        factory_a,
        init_data: std::env::var_os("PRIME_TEST_INIT_DATA").map(PathBuf::from),
        out_dir: out_dir.clone(),
        record: false,
    };
    let results = appsuite::run_suite(
        &cfg,
        std::env::var("PRIME_TEST_APPSUITE_ONLY").ok().as_deref(),
    )
    .unwrap();
    println!("{}", appsuite::summary(&results));
    println!("reports: {}", out_dir.display());
    for r in &results {
        assert!(r.as_expected(), "{}", r.text());
        assert_eq!(
            r.physical,
            appsuite::Verdict::NotVerified,
            "no physical evidence is recorded"
        );
    }
}
