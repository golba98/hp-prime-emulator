//! `ppl check` / `ppl run` on the Baseconv fixtures through the real
//! binary: scripted INPUT answers, `<ESC>` cancellation and exit codes.

use std::path::PathBuf;
use std::process::Command;

const PPL: &str = env!("CARGO_BIN_EXE_ppl");

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/baseconv")
        .join(name)
}

/// `(exit code, stdout, stderr)`.
fn ppl(args: &[&str]) -> (Option<i32>, String, String) {
    let out = Command::new(PPL).args(args).output().expect("run ppl");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn check_accepts_every_fixture() {
    for name in [
        "Baseconv_original.hpprgm",
        "Baseconv_corrected.hpprgm",
        "Baseconv_fixed.hpprgm",
    ] {
        let path = fixture(name);
        let (code, _, err) = ppl(&["check", path.to_str().unwrap()]);
        assert_eq!(code, Some(0), "{name}: {err}");
        assert!(
            err.contains("is valid PPL (1 function(s), 0 warning(s))"),
            "{err}"
        );
    }
}

#[test]
fn run_converts_a_typed_hex_string() {
    let path = fixture("Baseconv_fixed.hpprgm");
    let p = path.to_str().unwrap();
    for (answers, want) in [
        (["FF", "16", "10", "10"], "Final Result: 255"),
        (["7B.8", "16", "2", "10"], "Final Result: 01111011.1000"),
    ] {
        let mut args = vec!["run", p];
        for a in &answers {
            args.extend(["--input", a]);
        }
        let (code, out, err) = ppl(&args);
        assert_eq!(code, Some(0), "{answers:?}: {err}");
        assert!(out.lines().any(|l| l == want), "{answers:?}: {out}");
    }
}

#[test]
fn run_escape_cancels_cleanly() {
    let path = fixture("Baseconv_fixed.hpprgm");
    let (code, out, err) = ppl(&["run", path.to_str().unwrap(), "--input", "<ESC>"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(!out.contains("Final Result"), "{out}");
    assert!(!err.contains("runtime error"), "{err}");
}
