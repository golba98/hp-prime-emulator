//! Offline CLI contracts. These tests never open a physical HID connection.
use serde_json::{Value, json};
#[cfg(unix)]
use std::path::Path;
use std::process::{Command, Output, Stdio};
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ppl"))
        .args(["device"])
        .args(args)
        .env_remove("PPL_FIRMWARE")
        .env_remove("PPL_OS_ROM")
        .env_remove("PPL_FACTORY_A")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}
fn report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: {} stderr {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
#[test]
fn navigation_is_refused_without_usb_or_success_claims() {
    for action in [
        vec!["home"],
        vec!["programs"],
        vec!["enter"],
        vec!["esc"],
        vec!["up"],
        vec!["down"],
        vec!["select", "FIREWORKS"],
        vec!["edit", "FIREWORKS"],
        vec!["check", "FIREWORKS", "--dismiss"],
    ] {
        let mut args = vec!["ui"];
        args.extend(action);
        args.push("--json");
        let output = run(&args);
        let r = report(&output);
        assert_eq!(output.status.code(), Some(6), "{r}");
        assert_eq!(r["status"], "unsupported_capability");
        for field in [
            "uploaded",
            "execution_confirmed",
            "check_executed",
            "physical_execution_verified",
            "navigation_confirmed",
        ] {
            assert_eq!(r[field], false, "{r}");
        }
    }
}
#[test]
fn malformed_new_requests_have_machine_readable_errors() {
    for args in [
        vec!["programs", "show", "--json"],
        vec!["ui", "check", "--json"],
        vec!["diagnose", "x", "--engine", "native", "--json"],
        vec!["debug", "x", "--max-attempts", "0", "--json"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(report(&output)["status"], "invalid_input");
    }
}
#[test]
fn physical_requests_never_fall_back_to_emulation() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ZZT_LOCAL.hpprgm");
    std::fs::write(&file, "EXPORT ZZT_LOCAL() BEGIN RETURN 4; END;").unwrap();
    for args in [
        vec!["run-file", file.to_str().unwrap(), "--json"],
        vec![
            "diagnose",
            file.to_str().unwrap(),
            "--engine",
            "physical",
            "--json",
        ],
    ] {
        let output = run(&args);
        let r = report(&output);
        assert_eq!(output.status.code(), Some(6));
        assert_eq!(r["engine"], "physical_g1");
        assert_eq!(r["execution_state"], "not_requested");
        assert!(r["source"]["input_sha256"].as_str().is_some());
    }
}
#[test]
#[cfg(target_os = "linux")]
fn firmware_default_diagnosis_is_local_and_exposes_missing_runner() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("LOCAL.hpprgm");
    std::fs::write(&file, "export LOCAL() begin end;").unwrap();
    let output = run(&[
        "diagnose",
        file.to_str().unwrap(),
        "--firmware-runner",
        "/does-not-exist/prime-emulator",
        "--json",
    ]);
    let r = report(&output);
    assert_eq!(r["engine"], "firmware_emulator");
    assert_eq!(r["status"], "missing_firmware", "{r}");
    assert!(r["device"].is_null());
    assert_eq!(r["execution_confirmed"], false);
}
#[test]
#[cfg(target_os = "linux")]
fn debug_preserves_source_and_resumes_only_changed_revisions_with_same_limit() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("LOCAL.hpprgm");
    let artifacts = dir.path().join("attempts");
    let original = "EXPORT LOCAL()\r\nBEGIN END;\r\n";
    std::fs::write(&file, original).unwrap();
    let output = run(&[
        "debug",
        file.to_str().unwrap(),
        "--artifacts-dir",
        artifacts.to_str().unwrap(),
        "--max-attempts",
        "2",
        "--firmware-runner",
        "/does-not-exist/prime-emulator",
        "--json",
    ]);
    let r = report(&output);
    let id = r["session_id"].as_str().unwrap();
    assert_eq!(r["status"], "missing_firmware", "{r}");
    assert_eq!(r["attempts_used"], 1);
    assert_eq!(
        std::fs::read(r["source_snapshot"].as_str().unwrap()).unwrap(),
        original.as_bytes()
    );
    let output = run(&[
        "debug",
        file.to_str().unwrap(),
        "--artifacts-dir",
        artifacts.to_str().unwrap(),
        "--resume",
        id,
        "--json",
    ]);
    assert_eq!(report(&output)["status"], "refused");
    std::fs::write(&file, "EXPORT LOCAL() BEGIN RETURN 1; END;").unwrap();
    let output = run(&[
        "debug",
        file.to_str().unwrap(),
        "--artifacts-dir",
        artifacts.to_str().unwrap(),
        "--resume",
        id,
        "--max-attempts",
        "3",
        "--json",
    ]);
    assert_eq!(report(&output)["status"], "refused");
    let output = run(&[
        "debug",
        file.to_str().unwrap(),
        "--artifacts-dir",
        artifacts.to_str().unwrap(),
        "--resume",
        id,
        "--firmware-runner",
        "/does-not-exist/prime-emulator",
        "--json",
    ]);
    assert_eq!(report(&output)["attempts_used"], 2);
    std::fs::write(&file, "EXPORT LOCAL() BEGIN RETURN 2; END;").unwrap();
    let output = run(&[
        "debug",
        file.to_str().unwrap(),
        "--artifacts-dir",
        artifacts.to_str().unwrap(),
        "--resume",
        id,
        "--json",
    ]);
    assert_eq!(report(&output)["status"], "refused");
    let journal: Value =
        serde_json::from_slice(&std::fs::read(artifacts.join(id).join("session.json")).unwrap())
            .unwrap();
    assert_eq!(journal["attempts"].as_array().unwrap().len(), 2);
}
#[test]
fn export_never_overwrites_local_source_even_without_a_device() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("personal.hpprgm");
    std::fs::write(&path, "personal").unwrap();
    let output = run(&[
        "programs",
        "export",
        "P",
        "--out",
        path.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(report(&output)["status"], "refused");
    assert_eq!(std::fs::read_to_string(path).unwrap(), "personal");
}

#[cfg(unix)]
fn socket_reply(
    root: &Path,
    response: ppl_device::control::Response,
) -> (
    std::path::PathBuf,
    std::thread::JoinHandle<ppl_device::control::Request>,
) {
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let socket = root.join("control.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let req = ppl_device::control::read_message(&mut stream).unwrap();
        ppl_device::control::write_response(&mut stream, &response).unwrap();
        req
    });
    (socket, thread)
}
#[cfg(unix)]
#[test]
fn inventory_partial_is_not_promoted_by_the_cli_and_unknown_types_survive() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, thread) = socket_reply(
        dir.path(),
        ppl_device::control::Response {
            report: json!({"schema_version":1,"status":"inventory_incomplete","programs":[{"name":"REAL","size_bytes":10}],"unknown_objects":[{"object_type":99}],"inventory":{"complete":false},"error":{"kind":"inventory_incomplete","exit_code":9}}),
            payload: vec![],
        },
    );
    let output = run(&[
        "programs",
        "list",
        "--control-socket",
        socket.to_str().unwrap(),
        "--json",
    ]);
    let r = report(&output);
    assert_eq!(output.status.code(), Some(9));
    assert_eq!(r["inventory"]["complete"], false);
    assert_eq!(r["programs"][0]["name"], "REAL");
    assert_eq!(r["unknown_objects"][0]["object_type"], 99);
    assert!(matches!(
        thread.join().unwrap(),
        ppl_device::control::Request::Inventory
    ));
}
#[cfg(unix)]
#[test]
fn wrong_service_identity_prevents_source_request_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("P.hpprgm");
    let (socket, thread) = socket_reply(
        dir.path(),
        ppl_device::control::Response {
            report: json!({"status":"valid","device":{"serial_id":"other"},"error":null}),
            payload: vec![1, 2],
        },
    );
    let output = run(&[
        "programs",
        "export",
        "P",
        "--out",
        out.to_str().unwrap(),
        "--device",
        "expected",
        "--control-socket",
        socket.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(report(&output)["status"], "identity_mismatch");
    assert!(!out.exists());
    assert!(matches!(
        thread.join().unwrap(),
        ppl_device::control::Request::Status
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn genuine_device_diagnosis_distinguishes_local_valid_and_invalid_firmware_results() {
    let Some(firmware) = std::env::var_os("PRIME_TEST_FIRMWARE") else {
        eprintln!("SKIP physical-control local firmware integration: PRIME_TEST_FIRMWARE unset");
        return;
    };
    let factory =
        std::env::var_os("PRIME_TEST_FACTORY_A").expect("explicit firmware tests need factory A");
    let runner =
        std::env::var_os("PRIME_TEST_RUNNER").expect("explicit firmware tests need helper");
    let dir = tempfile::tempdir().unwrap();
    for (name, source, status, exit) in [
        (
            "ZZT_CHECK",
            "EXPORT ZZT_CHECK()\nBEGIN\nRETURN 4;\nEND;\n",
            "valid",
            0,
        ),
        (
            "ZZT_BAD",
            "EXPORT ZZT_BAD()\nBEGIN\nPRINT(\"bad\";\nEND;\n",
            "syntax_error",
            1,
        ),
    ] {
        let file = dir.path().join(format!("{name}.hpprgm"));
        std::fs::write(&file, source).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_ppl"));
        cmd.args(["device", "diagnose"])
            .arg(file)
            .args(["--json", "--firmware"])
            .arg(&firmware)
            .arg("--factory-a")
            .arg(&factory)
            .arg("--firmware-runner")
            .arg(&runner)
            .arg("--artifacts-dir")
            .arg(dir.path().join(name));
        if let Some(rom) = std::env::var_os("PRIME_TEST_OS_ROM") {
            cmd.arg("--os-rom").arg(rom);
        }
        if let Some(initial) = std::env::var_os("PRIME_TEST_INIT_DATA") {
            cmd.arg("--init-data").arg(initial);
        }
        let output = cmd.output().unwrap();
        let r = report(&output);
        assert_eq!(output.status.code(), Some(exit), "{r}");
        assert_eq!(r["status"], status);
        assert_eq!(r["engine"], "firmware_emulator");
        assert!(r["device"].is_null());
        assert_eq!(r["execution_confirmed"], false);
        assert_eq!(r["check_executed"], false);
        assert_eq!(
            r["source"]["engine_source_sha256"],
            r["firmware_report"]["backend"]["report"]["source_sha256"]
        );
        if status == "syntax_error" {
            assert!(!r["diagnostics"].as_array().unwrap().is_empty());
        }
    }
}

#[cfg(unix)]
#[test]
fn selected_service_is_checked_before_source_read_and_export_preserves_wire_bytes() {
    use ppl_device::control::{Request, Response};
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let socket = dir.path().join("selected.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let source = "EXPORT P()\r\nBEGIN PRINT(\"é\"); END;\r\n";
    let payload: Vec<_> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let expected = payload.clone();
    let thread = std::thread::spawn(move || {
        for phase in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let request: Request = ppl_device::control::read_message(&mut stream).unwrap();
            let response = if phase == 0 {
                assert!(matches!(request, Request::Status));
                Response {
                    report: json!({"schema_version":1,"status":"valid","error":null,"device":{"serial_id":"selected"}}),
                    payload: vec![],
                }
            } else {
                assert!(matches!(request,Request::Program{ref name} if name=="P"));
                Response {
                    report: json!({"schema_version":1,"status":"valid","error":null,"device":{"serial_id":"selected"},"program":"P","object_type":{"code":6},"encoding":"utf-16le","source_length_bytes":payload.len(),"sha256":ppl_compat::sha256_hex(&payload),"source":source,"source_sha256":ppl_compat::sha256_hex(source.as_bytes())}),
                    payload: payload.clone(),
                }
            };
            ppl_device::control::write_response(&mut stream, &response).unwrap();
        }
    });
    let path = dir.path().join("P.hpprgm");
    let output = run(&[
        "programs",
        "export",
        "P",
        "--out",
        path.to_str().unwrap(),
        "--device",
        "selected",
        "--control-socket",
        socket.to_str().unwrap(),
        "--json",
    ]);
    let r = report(&output);
    assert_eq!(output.status.code(), Some(0), "{r}");
    assert_eq!(r["exported"], true);
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    let sidecar: Value = serde_json::from_slice(
        &std::fs::read(path.with_extension("hpprgm.metadata.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(sidecar["source"], source);
    assert_eq!(sidecar["sha256"], r["sha256"]);
    thread.join().unwrap();
}
