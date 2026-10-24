//! Guarded listener installation and fixed, non-destructive hardware experiments.
use super::device::DeviceOpts;
use super::remote::{
    Failure, Result, control, evidence, input, refuse_pending, transact, unsupported,
};
use super::remote_state::{Capabilities, Identity, Listener, Store};
use ppl_device::{FileType, Prime, ReadBack};
use serde_json::{Value, json};

pub(super) fn install(
    prime: &mut Prime,
    store: &Store,
    id: &Identity,
    new_session: bool,
    out: &mut Value,
) -> Result<()> {
    refuse_pending(store)?;
    if store.listener_path().exists() && !new_session {
        return Err(Failure {code:8,kind:"existing_installation",message:"listener already recorded; inspect `listener status` and start that program; installation never replaces it".into()});
    }
    if store.listener_path().exists() {
        let previous: Listener = store.read(&store.listener_path())?;
        store.save(
            &store
                .dir
                .join(format!("listener-{}.json", previous.session)),
            &previous,
        )?;
    }
    let program = ppl_device::fresh_test_name();
    let session = program.trim_start_matches("ZZT_").to_string();
    let source = ppl_device::listener::source(&program, &session)?;
    super::remote_input::declarations(&source).map_err(input)?;
    let (_, diags) = ppl_syntax::parse(&source);
    if diags.has_errors() {
        return Err(input("generated listener failed local syntax validation"));
    }
    let mut listener = Listener {
        schema_version: 1,
        device: id.clone(),
        program: program.clone(),
        session: session.clone(),
        source_sha256: ppl_compat::sha256_hex(source.as_bytes()),
        source: source.clone(),
        uploaded: false,
        heartbeat: Some(format!("ZZT_H{session}")),
        capabilities: Capabilities::default(),
        verification_requests: vec![],
    };
    let candidate_path = store
        .dir
        .join(format!("listener-{}.json", listener.session));
    store.save(&candidate_path, &listener)?;
    std::fs::write(store.dir.join(format!("{program}.hpprgm")), &source)?;
    out["listener_program"] = json!(program);
    evidence(out, candidate_path);
    let heartbeat = listener
        .heartbeat
        .as_deref()
        .expect("new listener heartbeat");
    let guard = prime.preflight_new_names(
        &[(FileType::Program, &program), (FileType::Note, heartbeat)],
        Some(&control()),
    )?;
    out["upload_state"] = json!("unverified");
    let report = prime.send_new_program(guard, &source)?;
    if report.readback != ReadBack::Matches {
        return Err(Failure {
            code: 9,
            kind: "upload_unverified",
            message: format!(
                "listener upload unverified: {:?}; inspect it, do not reinstall blindly",
                report.readback
            ),
        });
    }
    listener.uploaded = true;
    store.save(&store.listener_path(), &listener)?;
    out["uploaded"] = json!(true);
    out["upload_state"] = json!("verified");
    out["manual_action_required"] = json!(format!(
        "On the calculator, open Program Catalog and Run {program} once. Then run `ppl device listener probe --json`. Esc exits the listener."
    ));
    out["capabilities"] = json!(listener.capabilities);
    Ok(())
}

pub(super) fn upload_program(
    prime: &mut Prime,
    store: &Store,
    name: &str,
    source: &str,
    out: &mut Value,
) -> Result<()> {
    let path = store.dir.join(format!("program-{name}.hpprgm"));
    if path.exists() {
        return Err(input(
            "program name was already reserved locally; generate a fresh ZZT_ name",
        ));
    }
    std::fs::create_dir_all(&store.dir)?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    file.write_all(source.as_bytes())?;
    file.sync_all()?;
    evidence(out, path);
    let guard = prime.preflight_new_names(&[(FileType::Program, name)], Some(&control()))?;
    out["upload_state"] = json!("unverified");
    let upload = prime.send_new_program(guard, source)?;
    if upload.readback != ReadBack::Matches {
        return Err(Failure {
            code: 9,
            kind: "upload_unverified",
            message: "program upload unverified; no execution requested and no automatic resend"
                .into(),
        });
    }
    out["uploaded"] = json!(true);
    out["upload_state"] = json!("verified");
    Ok(())
}

pub(super) fn probe(
    prime: &mut Prime,
    store: &Store,
    listener: &mut Listener,
    opts: &DeviceOpts,
    out: &mut Value,
) -> Result<()> {
    for _ in 0..2 {
        let challenge = ppl_device::fresh_test_name();
        let reply = transact(prime, store, listener, &challenge, true, opts, out)?;
        if reply.result_type != 2 || reply.text != format!("PONG:{challenge}") {
            return Err(Failure {
                code: 9,
                kind: "challenge_mismatch",
                message: "calculator did not echo the fresh challenge".into(),
            });
        }
        listener
            .verification_requests
            .push(out["request_id"].as_str().expect("id").into());
    }
    listener.capabilities.mailbox = true;
    store.save(&store.listener_path(), listener)?;
    out["capabilities"] = json!(listener.capabilities);
    out["execution_state"] = json!("mailbox_verified");
    Ok(())
}
fn expect_number(reply: &ppl_device::remote::Reply, expected: &str) -> Result<()> {
    if reply.result_type != 0 || reply.text != expected {
        return Err(Failure {
            code: 9,
            kind: "unexpected_result",
            message: format!(
                "expected physical numeric result {expected}, got type {}: {}",
                reply.result_type, reply.text
            ),
        });
    }
    Ok(())
}
pub(super) fn verify(
    prime: &mut Prime,
    store: &Store,
    listener: &mut Listener,
    opts: &DeviceOpts,
    out: &mut Value,
) -> Result<()> {
    if !listener.capabilities.mailbox {
        return Err(unsupported(
            "first run `ppl device listener probe` to verify two physical echo challenges",
        ));
    }
    for (expr, expected) in [("2+2", "4"), ("6*7", "42")] {
        let reply = transact(prime, store, listener, expr, false, opts, out)?;
        expect_number(&reply, expected)?;
        listener
            .verification_requests
            .push(out["request_id"].as_str().expect("id").into());
    }
    // This request is deliberately a runtime error; it must have a terminal
    // response so that another request can safely follow.
    match transact(prime, store, listener, "1/0", false, opts, out) {
        Err(e) if e.code == 3 => (),
        Err(e) => return Err(e),
        Ok(_) => {
            return Err(Failure {
                code: 9,
                kind: "missing_runtime_error",
                message: "1/0 did not produce the expected trapped error".into(),
            });
        }
    }
    let reply = transact(prime, store, listener, "7+8", false, opts, out)?;
    expect_number(&reply, "15")?;
    listener.capabilities.expressions = true;
    store.save(&store.listener_path(), listener)?;
    let name = ppl_device::fresh_test_name();
    let source = format!("EXPORT {name}(A,B)\nBEGIN\n RETURN A+B;\nEND;\n");
    upload_program(prime, store, &name, &source, out)?;
    let reply = transact(
        prime,
        store,
        listener,
        &format!("{name}(10,20)"),
        false,
        opts,
        out,
    )?;
    expect_number(&reply, "30")?;
    listener
        .verification_requests
        .push(out["request_id"].as_str().expect("id").into());
    listener.capabilities.program_calls = true;
    listener.capabilities.live_upload = true;
    store.save(&store.listener_path(), listener)?;
    out["capabilities"] = json!(listener.capabilities);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppl_device::{Detected, DeviceError, Transport};
    struct Disconnected;
    impl Transport for Disconnected {
        fn write(&mut self, _: &[u8]) -> ppl_device::Result<usize> {
            Err(DeviceError::Hid("disconnected".into()))
        }
        fn read_timeout(&mut self, _: &mut [u8], _: i32) -> ppl_device::Result<usize> {
            Ok(0)
        }
    }
    #[test]
    fn failed_new_installation_preserves_current_installation() {
        let root = std::env::temp_dir().join(format!(
            "ppl-install-test-{}",
            ppl_device::fresh_test_name()
        ));
        let identity = Identity {
            model: "mock".into(),
            usb_pid: 0x1541,
            firmware_build: Some(15157),
            boot_version: None,
            serial_id: "mock".into(),
        };
        let store = Store::new(&root, &identity);
        let current = Listener {
            schema_version: 1,
            device: identity.clone(),
            program: "ZZT_OLD".into(),
            session: "ABC123".into(),
            source: "old".into(),
            source_sha256: "old".into(),
            uploaded: true,
            heartbeat: None,
            capabilities: Capabilities::default(),
            verification_requests: vec![],
        };
        store.save(&store.listener_path(), &current).unwrap();
        let detected = Detected {
            vendor_id: 0x03f0,
            product_id: 0x1541,
            model: "mock",
            path: "mock".into(),
            manufacturer: None,
            product: None,
            serial: None,
            release: 0,
            interface: 0,
        };
        let mut prime = Prime::with_transport(detected, Box::new(Disconnected));
        let mut output = json!({"evidence_paths":[]});
        assert!(install(&mut prime, &store, &identity, true, &mut output).is_err());
        let preserved: Listener = store.read(&store.listener_path()).unwrap();
        assert_eq!(preserved.program, "ZZT_OLD");
        assert!(preserved.uploaded);
        std::fs::remove_dir_all(root).unwrap();
    }
}
