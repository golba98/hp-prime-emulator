//! `ppl check` (the real binary) and the IDE (a file opened the way the
//! editor opens it, analysed the way the Problems panel analyses it) must
//! report the same findings: same severity, code, line and column.

use std::path::{Path, PathBuf};
use std::process::Command;

use ppl_gui::ide::diagnostics::check_source;
use ppl_gui::ide::editor::EditorTab;
use ppl_syntax::Severity;

const PPL: &str = env!("CARGO_BIN_EXE_ppl");

type Finding = (String, String, u32, u32);

/// Findings printed by `ppl check FILE`.
fn cli_findings(path: &Path) -> Vec<Finding> {
    let out = Command::new(PPL)
        .args(["check", path.to_str().unwrap()])
        .env("NO_COLOR", "1")
        .output()
        .expect("run ppl");
    let text = String::from_utf8_lossy(&out.stderr);
    let mut found = Vec::new();
    let mut head: Option<(String, String)> = None;
    for line in text.lines() {
        if let Some((sev, rest)) = line.split_once('[') {
            if ["error", "warning", "note"].contains(&sev) {
                let code = rest.split(']').next().unwrap().to_string();
                head = Some((sev.to_string(), code));
                continue;
            }
        }
        if let (Some(loc), Some((sev, code))) = (line.trim().strip_prefix("--> "), head.take()) {
            let mut parts = loc.rsplitn(3, ':');
            let col = parts.next().unwrap().parse().unwrap();
            let ln = parts.next().unwrap().parse().unwrap();
            found.push((sev, code, ln, col));
        }
    }
    found.sort();
    found
}

/// Findings the IDE shows for the file after opening it.
fn ide_findings(path: &Path) -> Vec<Finding> {
    let tab = EditorTab::open(path.to_path_buf()).expect("open");
    let mut found: Vec<Finding> = check_source(&tab.title, &tab.content)
        .into_iter()
        .map(|d| {
            let sev = match d.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Note => "note",
            };
            (sev.to_string(), d.code, d.line as u32, d.col as u32)
        })
        .collect();
    found.sort();
    found
}

fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gui_cli_agreement");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn utf16le_bom(text: &str) -> Vec<u8> {
    let mut b = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        b.extend(u.to_le_bytes());
    }
    b
}

#[test]
fn cli_and_ide_report_identical_findings() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut paths: Vec<PathBuf> = [
        "baseconv/Baseconv_original.hpprgm",
        "baseconv/Baseconv_corrected.hpprgm",
        "baseconv/Baseconv_fixed.hpprgm",
        "FIREWORKS.hpprgm",
        "torture_test.hpprgm",
    ]
    .iter()
    .map(|f| fixtures.join(f))
    .collect();
    let samples: [(&str, &str); 6] = [
        (
            "unclosed_if.ppl",
            "EXPORT DEMO()\nBEGIN\n  LOCAL a;\n  IF a == 1 THEN\n",
        ),
        (
            "missing_value.ppl",
            "EXPORT DEMO()\nBEGIN\n  LOCAL a;\n  a := ;\nEND;\n",
        ),
        (
            "hello.ppl",
            "EXPORT DEMO()\nBEGIN\n  PRINT(\"Hello\");\nEND;\n",
        ),
        (
            "unicode.ppl",
            "EXPORT UNI()\nBEGIN\n\tLOCAL a; PRINT(\"₂é→\"); a := ;\n  x ▶ a;\nEND;\n",
        ),
        (
            "warnings.ppl",
            "EXPORT EV()\nBEGIN\n  LOCAL q;\n  RETURN EVAL(\"1+1\");\nEND;\n\
             EXPORT DUP()\nBEGIN\n  LOCAL b; LOCAL b;\n  INPUT({{b,[2],{1,2,3}}});\nEND;\n",
        ),
        (
            "crlf.ppl",
            "EXPORT CR()\r\nBEGIN\r\n  RETURN zz;\r\nEND;\r\n",
        ),
    ];
    for (name, text) in samples {
        paths.push(temp_file(name, text.as_bytes()));
    }
    // The same source stored as UTF-16LE (as the calculator saves it).
    paths.push(temp_file(
        "unicode_utf16.hpprgm",
        &utf16le_bom(samples[3].1),
    ));

    let mut total = 0;
    for path in &paths {
        let cli = cli_findings(path);
        let ide = ide_findings(path);
        assert_eq!(ide, cli, "{}", path.display());
        total += cli.len();
    }
    // The comparison is not vacuous.
    assert!(total >= 10, "only {total} findings compared");
}
