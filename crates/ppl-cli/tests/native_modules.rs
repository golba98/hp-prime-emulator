use std::{fs, process::Command};
fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ppl"))
        .env("PPL_ENGINE", "native")
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn cli_links_modules_and_checks_the_same_catalog() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("ROOT.ppl");
    let dep = d.path().join("B.ppl");
    let bad = d.path().join("BAD.ppl");
    fs::write(&root, "EXPORT TOP() BEGIN RETURN LEAF(); END;").unwrap();
    fs::write(&dep, "EXPORT LEAF() BEGIN RETURN 42; END;").unwrap();
    fs::write(&bad, "EXPORT BAD() BEGIN RETURN 1+; END;").unwrap();
    let module = format!("B={}", dep.display());
    let unrelated = format!("BAD={}", bad.display());
    for action in ["run", "check"] {
        let out = run(&[
            action,
            root.to_str().unwrap(),
            "--engine",
            "native",
            "--json",
            "--module",
            &module,
            "--module",
            &unrelated,
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(report["status"], "success");
        if action == "run" {
            assert_eq!(
                report["return_value"],
                serde_json::json!({"type":"real","value":"42"})
            );
        }
    }
}
#[test]
fn module_output_alias_is_refused_without_overwriting_source() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("ROOT.ppl");
    let dep = d.path().join("B.ppl");
    let body = "EXPORT LEAF() BEGIN RETURN 42; END;";
    fs::write(&root, "EXPORT TOP() BEGIN RETURN LEAF(); END;").unwrap();
    fs::write(&dep, body).unwrap();
    let module = format!("B={}", dep.display());
    let out = run(&[
        "run",
        root.to_str().unwrap(),
        "--engine",
        "native",
        "--json",
        "--module",
        &module,
        "--report",
        dep.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert_eq!(fs::read_to_string(dep).unwrap(), body);
}
#[test]
fn foreign_runtime_error_has_callee_source_identity() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("ROOT.ppl");
    let dep = d.path().join("B.ppl");
    fs::write(&root, "EXPORT TOP() BEGIN RETURN LEAF(); END;").unwrap();
    fs::write(&dep, "EXPORT LEAF()\nBEGIN\n RETURN 1/0;\nEND;").unwrap();
    let module = format!("B={}", dep.display());
    let out = run(&[
        "run",
        root.to_str().unwrap(),
        "--engine",
        "native",
        "--json",
        "--module",
        &module,
    ]);
    assert!(!out.status.success());
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        report["runtime_errors"][0]["raw_position"]["module"]["name"],
        "B"
    );
    assert_eq!(report["runtime_errors"][0]["line"], 3);
}
