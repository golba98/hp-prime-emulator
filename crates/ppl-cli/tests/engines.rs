use std::process::Command;
const PPL: &str = env!("CARGO_BIN_EXE_ppl");
/// The CLI with the default engine pinned to native, independent of any
/// firmware configured on the developer's machine (config file or env).
fn native_ppl() -> Command {
    let mut c = Command::new(PPL);
    c.env("PPL_ENGINE", "native")
        .env("PPL_CONFIG", "/nonexistent/ppl-test-config.toml");
    for asset in [
        "PPL_FIRMWARE",
        "PPL_OS_ROM",
        "PPL_FACTORY_A",
        "PPL_FIRMWARE_RUNNER",
        "PPL_FIRMWARE_INIT_DATA",
    ] {
        c.env_remove(asset);
    }
    c
}
fn invoke(args: &[&str]) -> (i32, String, String) {
    let o = native_ppl().args(args).output().unwrap();
    (
        o.status.code().unwrap(),
        String::from_utf8(o.stdout).unwrap(),
        String::from_utf8(o.stderr).unwrap(),
    )
}
#[test]
fn current_and_legacy_numeric_models_work_through_both_production_cli_paths() {
    let source = "EXPORT DIGITS(X) BEGIN PRINT((X+1)-X); RETURN 1/3; END;";
    let (status, output, error) = invoke(&[
        "run",
        "--code",
        source,
        "--numeric-version",
        "15751",
        "--args",
        "1234567890123456789",
    ]);
    assert_eq!(status, 0, "{error}");
    assert_eq!(output.trim(), "1");
    let (status, output, error) = invoke(&[
        "run",
        "--code",
        source,
        "--numeric-version",
        "15157",
        "--args",
        "1234567890123456789",
    ]);
    assert_eq!(status, 0, "{error}");
    assert_eq!(output.trim(), "0");
    for argument in [
        "[1234567890123456789]",
        r#"[{"type":"real","value":"1234567890123456789"}]"#,
    ] {
        let (status, output, error) = invoke(&[
            "run",
            "--code",
            source,
            "--engine",
            "native",
            "--numeric-version",
            "15751",
            "--args",
            argument,
            "--json",
        ]);
        assert_eq!(status, 0, "{error}");
        let report: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(
            report["return_value"],
            serde_json::json!({"type":"rational","negative":false,"numerator":"1","denominator":"3"})
        );
        assert_eq!(report["output"][0]["text"], "1");
    }
    let (status, output, _) = invoke(&[
        "run",
        "--code",
        source,
        "--engine",
        "firmware",
        "--numeric-version",
        "15751",
        "--json",
    ]);
    assert_eq!(status, 8, "{output}");
    let report: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(report["output"], serde_json::json!([]));
    assert!(
        report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("native VM controls")
    );
}
#[test]
fn old_and_explicit_native_arguments_remain_distinct() {
    let code = "EXPORT ECHO(X) BEGIN RETURN X; END;";
    let (c, _, e) = invoke(&["run", "-c", code, "--args", "[1,2]"]);
    assert_eq!(c, 0);
    assert!(e.contains("returned [1,2]"));
    let (c, o, _) = invoke(&[
        "run", "-c", code, "--engine", "native", "--args", "[7]", "--json",
    ]);
    assert_eq!(c, 0);
    let r: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert_eq!(r["return_value"]["value"], "7");
    let (c, o, _) = invoke(&[
        "run",
        "-c",
        code,
        "--engine",
        "native",
        "--args-format",
        "ppl",
        "--args",
        "[1,2]",
        "--json",
    ]);
    assert_eq!(c, 0);
    let r: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert_eq!(r["return_value"]["type"], "matrix");
}
#[test]
fn json_is_one_document_and_explicit_entry_is_respected() {
    let code = "EXPORT INIT() BEGIN RETURN 1; END; EXPORT MAIN(N) BEGIN PRINT(\"é\\\"\"); RETURN N*N; END;";
    let (c, o, _) = invoke(&[
        "run", "-c", code, "--engine", "native", "--entry", "MAIN", "--args", "[7]", "--json",
    ]);
    assert_eq!(c, 0, "{o}");
    let r: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert_eq!(r["entry"], "MAIN");
    assert_eq!(r["return_value"]["value"], "49");
    assert_eq!(r["output"][0]["kind"], "print");
}
#[test]
fn missing_assets_and_unsupported_values_never_fall_back() {
    let code = "EXPORT HELLO() BEGIN PRINT(\"NATIVE FALLBACK\"); END;";
    for (extra, expected) in [
        (vec![], if cfg!(target_os = "linux") { 11 } else { 8 }),
        (vec!["--args", "[\"x\"]"], 8),
    ] {
        let mut a = vec!["run", "-c", code, "--engine", "firmware", "--json"];
        a.extend(extra);
        let (c, o, _) = invoke(&a);
        assert_eq!(c, expected, "{o}");
        let r: serde_json::Value = serde_json::from_str(&o).unwrap();
        assert_eq!(r["engine"], "firmware");
        assert_eq!(r["output"], serde_json::json!([]));
    }
}
#[test]
fn native_errors_keep_legacy_exit_codes() {
    for (code, extra, expected) in [
        ("EXPORT FUNCASE() BEGIN A:=; END;", vec![], 1),
        ("EXPORT FUNCASE() BEGIN RETURN 1/0; END;", vec![], 3),
        (
            "EXPORT FUNCASE() BEGIN RETURN 1; END;",
            vec!["--entry", "NOPE"],
            2,
        ),
    ] {
        let mut a = vec!["run", "-c", code, "--engine", "native", "--json"];
        a.extend(extra);
        let (c, o, _) = invoke(&a);
        assert_eq!(c, expected, "{o}");
        serde_json::from_str::<serde_json::Value>(&o).unwrap();
    }
}
#[test]
fn bad_engine_is_usage_and_json() {
    let (c, o, _) = invoke(&["check", "--engine", "fake", "--json", "-c", "x"]);
    assert_eq!(c, 2);
    let r: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert_eq!(r["status"], "invalid_request");
}
#[test]
fn source_is_not_overwritten_by_reports_and_pngs() {
    let d = tempfile::tempdir().unwrap();
    let source = d.path().join("HELLO.ppl");
    let text = b"EXPORT HELLO() BEGIN RETURN 7; END;";
    std::fs::write(&source, text).unwrap();
    for flags in [
        vec!["--engine", "native", "--report"],
        vec!["--engine", "native", "--png"],
        vec!["--png"],
    ] {
        let mut c = native_ppl();
        c.arg("run").arg(&source).args(flags).arg(&source);
        let o = c.output().unwrap();
        assert_eq!(o.status.code(), Some(5));
        assert_eq!(std::fs::read(&source).unwrap(), text);
    }
}
#[test]
fn native_works_outside_checkout_and_ignores_firmware_environment() {
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("HELLO é.ppl");
    std::fs::write(&file, "EXPORT HELLO() BEGIN RETURN 7; END;").unwrap();
    let o = native_ppl()
        .current_dir(d.path())
        .env("PPL_FIRMWARE_RUNNER", "/must/not/be/executed")
        .env("PPL_FIRMWARE", "/must/not/be/read")
        .args(["run", "--engine", "native", "--json"])
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    let r: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(r["entry"], "HELLO");
    assert_eq!(r["return_value"]["value"], "7");
}
#[test]
fn invalid_containers_and_extreme_malformed_arguments_are_structured() {
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("BAD.hpprgm");
    std::fs::write(&file, [0x7c, 0x61, 0x8a, 0xb2, 0, 0, 0, 0]).unwrap();
    for engine in ["native", "firmware"] {
        let o = native_ppl()
            .args(["check", "--engine", engine, "--json"])
            .arg(&file)
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(5));
        serde_json::from_slice::<serde_json::Value>(&o.stdout).unwrap();
    }
    let (c, o, _) = invoke(&[
        "run",
        "-c",
        "EXPORT HELLO() BEGIN END;",
        "--engine",
        "firmware",
        "--args",
        r#"[{"type":"real","value":"7","extra":1e400}]"#,
        "--json",
    ]);
    // On non-Linux platforms the firmware backend reports platform
    // unavailability before it can validate firmware-specific arguments.
    let result: serde_json::Value = serde_json::from_str(&o).unwrap();
    if cfg!(target_os = "linux") {
        assert_eq!(c, 7, "{o}");
        assert_eq!(result["status"], "invalid_arguments");
    } else {
        assert_eq!(c, 8, "{o}");
        assert_eq!(result["status"], "unsupported");
        assert_eq!(result["error"]["phase"], "platform");
    }
}
#[test]
fn native_json_bytecode_limits_and_filename_entry() {
    let (c, o, _) = invoke(&[
        "check",
        "-c",
        "EXPORT FUNCASE() BEGIN RETURN 1; END;",
        "--engine",
        "native",
        "--dump-bytecode",
        "--json",
    ]);
    assert_eq!(c, 0);
    let r: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert!(
        r["backend"]["disassembly"]
            .as_str()
            .unwrap()
            .contains("FUNCASE")
    );
    let (c, o, _) = invoke(&[
        "run",
        "-c",
        "EXPORT SPIN() BEGIN WHILE 1 DO END; END;",
        "--engine",
        "native",
        "--max-steps",
        "20",
        "--json",
    ]);
    assert_eq!(c, 4, "{o}");
    let r: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert_eq!(r["status"], "timed_out");
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/m8/BSTVisualizer.ppl");
    for extra in [vec![], vec!["--engine", "native", "--json"]] {
        let out = native_ppl()
            .arg("run")
            .arg(&file)
            .args(&extra)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
        if extra.is_empty() {
            assert!(
                String::from_utf8(out.stderr)
                    .unwrap()
                    .contains("BSTVisualizer returned 333")
            );
        } else {
            let r: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(r["entry"], "BSTVisualizer");
            assert_eq!(r["return_value"]["value"], "333");
        }
    }
}
#[test]
fn native_square_inputs_and_inline_filename_selection() {
    for (a, n) in [("[7]", "49"), ("[3]", "9"), ("[0]", "0"), ("[-4]", "16")] {
        let (c, o, _) = invoke(&[
            "run",
            "-c",
            "EXPORT SQUARE(N) BEGIN RETURN N*N; END;",
            "--engine",
            "native",
            "--entry",
            "SQUARE",
            "--args",
            a,
            "--json",
        ]);
        assert_eq!(c, 0);
        let r: serde_json::Value = serde_json::from_str(&o).unwrap();
        assert_eq!(r["return_value"]["value"], n);
    }
    let (c, o, _) = invoke(&[
        "run",
        "TARGET.ppl",
        "-c",
        "EXPORT INIT() BEGIN RETURN 1; END; EXPORT TARGET() BEGIN RETURN 7; END;",
        "--engine",
        "native",
        "--json",
    ]);
    assert_eq!(c, 0);
    let r: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert_eq!(r["entry"], "TARGET");
}
#[test]
fn implicit_and_explicit_native_share_utf16be_and_container_validation() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("HELLO.ppl");
    let text = "EXPORT HELLO()\r\nBEGIN\r\nRETURN 7;\r\nEND;";
    let bytes: Vec<u8> = [254, 255]
        .into_iter()
        .chain(text.encode_utf16().flat_map(u16::to_be_bytes))
        .collect();
    std::fs::write(&path, bytes).unwrap();
    for flags in [vec![], vec!["--engine", "native"]] {
        let o = native_ppl()
            .arg("check")
            .arg(&path)
            .args(flags)
            .output()
            .unwrap();
        assert_eq!(
            o.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

#[test]
fn string_version_is_explicit_in_both_native_routes_and_refused_for_firmware() {
    let code = "EXPORT FORMATVERSION() BEGIN RETURN {STRING(π,7,12),STRING(-7/3,15,3)}; END;";
    let (status, _, error) = invoke(&["run", "-c", code, "--string-format-version", "15157"]);
    assert_eq!(status, 0, "{error}");
    assert!(
        error.contains("312689/99532") && error.contains("−2−1/3"),
        "{error}"
    );
    let (status, output, error) = invoke(&[
        "run",
        "-c",
        code,
        "--engine",
        "native",
        "--json",
        "--string-format-version",
        "15157",
    ]);
    assert_eq!(status, 0, "{error}");
    let report: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(report["return_value"]["value"][0]["value"], "312689/99532");
    assert_eq!(report["return_value"]["value"][1]["value"], "−2−1/3");
    for version in ["15157", "15751"] {
        let (status, output, _) = invoke(&[
            "run",
            "-c",
            code,
            "--engine",
            "firmware",
            "--json",
            "--string-format-version",
            version,
        ]);
        assert_eq!(status, 8, "{output}");
        assert!(output.contains("native VM controls"), "{output}");
    }
    assert_ne!(
        invoke(&["run", "-c", code, "--string-format-version", "12345"]).0,
        0
    );
}

#[test]
fn both_native_cli_paths_preserve_actual_mouse_packet_shape_and_word_metadata() {
    let source = "EXPORT MOUSECLI() BEGIN LOCAL m:=MOUSE; RETURN {SIZE(m),SIZE(m(1)),B→R(m(1,1)),B→R(m(1,2)),GETBITS(m(1,1)),B→R(GETBASE(m(1,1))),GETBITS(m(1,5))}; END;";
    let (code, _, error) = invoke(&["run", "-c", source, "--keys", "touch:100:50"]);
    assert_eq!(code, 0, "{error}");
    assert!(
        error
            .replace(' ', "")
            .contains("returned{2,5,100,50,-16,0,16}"),
        "{error}"
    );
    let (code, output, error) = invoke(&[
        "run",
        "-c",
        source,
        "--keys",
        "touch:100:50",
        "--engine",
        "native",
        "--json",
    ]);
    assert_eq!(code, 0, "{output} {error}");
    let report: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(report["status"], "success");
    assert_eq!(
        report["return_value"]["value"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["value"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["2", "5", "100", "50", "-16", "0", "16"]
    );
}
