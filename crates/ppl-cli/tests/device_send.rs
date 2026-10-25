//! `ppl device send` argument, naming and size handling. Every case here
//! stops before USB is opened (invalid input, unreadable file, oversize
//! payload, or a declined name confirmation on empty stdin).
use std::process::{Command, Output, Stdio};

fn send(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(["device", "send"])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn ordinary_name_is_accepted_and_reaches_the_confirmation_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("FIREWORKS.hpprgm");
    std::fs::write(&file, "EXPORT FIREWORKS()\nBEGIN\nEND;\n").unwrap();
    let out = send(&[
        "FIREWORKS",
        file.to_str().unwrap(),
        "--runs-dir",
        dir.path().to_str().unwrap(),
    ]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert!(so.contains("About to create program `FIREWORKS`"), "{so}");
    assert!(!so.contains("result note"), "{so}");
    assert!(!se.contains("test object"), "{se}");
    // empty stdin declines the confirmation: refused before USB, exit 2
    assert_eq!(out.status.code(), Some(2), "{se}");
    assert!(se.contains("nothing was sent"), "{se}");
}

#[test]
fn invalid_names_are_usage_errors_that_do_not_mention_test_objects() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("p.hpprgm");
    std::fs::write(&file, "EXPORT P()\nBEGIN\nEND;\n").unwrap();
    for bad in ["1BAD", "MY PROG", "../x", "ABCDEFGHIJKLMNOPQ", "Home"] {
        let out = send(&[bad, file.to_str().unwrap()]);
        let se = text(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{bad}: {se}");
        assert!(se.contains("not a valid program name"), "{bad}: {se}");
        assert!(!se.contains("test object"), "{bad}: {se}");
    }
}

#[test]
fn missing_file_is_reported_as_such() {
    let out = send(&["FIREWORKS", "/nonexistent/FIREWORKS.hpprgm"]);
    let se = text(&out.stderr);
    assert_eq!(out.status.code(), Some(5), "{se}");
    assert!(se.contains("cannot read"), "{se}");
}

#[test]
fn oversize_payload_is_refused_before_the_prompt() {
    let example = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/FIREWORKS.hpprgm"
    );
    let out = send(&["FIREWORKS", example]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(2), "{se}");
    assert!(se.contains("silently drops"), "{se}");
    assert!(
        !so.contains("About to create"),
        "refused before the prompt: {so}"
    );
}

#[test]
fn test_prefix_workflow_and_note_rules_are_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.hpprgm");
    std::fs::write(&file, "EXPORT T()\nBEGIN\nEND;\n").unwrap();
    let out = send(&["ZZT_FIREWORKS", file.to_str().unwrap()]);
    let so = text(&out.stdout);
    assert!(so.contains("result note `ZZT_FIREWORKSR`"), "{so}");
    // a lower-case prefix is still held to the strict test-object rules
    let out = send(&["zzt_abc", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("test object"));
    // notes remain test-object only
    let out = send(&["FIREWORKS", file.to_str().unwrap(), "--note"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("test object"));
}

#[test]
fn help_describes_ordinary_names() {
    let out = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(["device", "send", "--help"])
        .output()
        .unwrap();
    let h = text(&out.stdout);
    assert!(
        h.contains("ppl device send FIREWORKS FIREWORKS.hpprgm"),
        "{h}"
    );
    assert!(!h.contains("whose name starts with ZZT_"), "{h}");
    assert!(h.contains("15,928"), "{h}");
}
