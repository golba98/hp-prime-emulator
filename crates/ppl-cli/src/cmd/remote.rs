//! Physical execution CLI and bounded listener experiments.
use super::device::{DeviceAction, DeviceOpts};
use super::remote_experiment::{install, probe, upload_program, verify};
use super::remote_state::{Capabilities, Identity, Journal, Listener, Store};
use clap::Subcommand;
use ppl_device::{ControlObject, DeviceError, FileType, Prime, Probe};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Subcommand, Debug)]
pub enum ListenerAction {
    /// Install a fresh experimental ZZT_ listener. Requires one manual start.
    Install {
        /// Preserve the previous installation and create a fresh listener program.
        #[arg(long)]
        new_session: bool,
    },
    /// Bounded pre-owned-note + screenshot experiment; no PPL execution mode.
    FixedMailboxProbe {
        /// Reuse a previously verified fixed receiver; it must be stopped first.
        #[arg(long)]
        resume: Option<String>,
    },
    /// Send two harmless echo challenges to the already running listener.
    Probe,
    /// Verify evaluation, live program upload/call, and runtime-error recovery.
    Verify,
    /// Close only an unresolved harmless echo probe; never cancels an evaluation.
    AbandonProbe { request_id: String },
    /// Read local capability evidence and inspect the installed listener.
    Status,
}

#[derive(Debug)]
pub(super) struct Failure {
    pub(super) code: u8,
    pub(super) kind: &'static str,
    pub(super) message: String,
}
pub(super) type Result<T> = std::result::Result<T, Failure>;
impl From<DeviceError> for Failure {
    fn from(e: DeviceError) -> Self {
        let (code, kind) = match &e {
            DeviceError::Busy(_) => (8, "busy"),
            DeviceError::Timeout(_) => (7, "timeout"),
            DeviceError::Protocol(_) => (9, "invalid_evidence"),
            DeviceError::Refused(_) => (2, "refused"),
            DeviceError::Inconclusive(_) => (5, "inconclusive"),
            DeviceError::NotFound => (5, "not_found"),
            DeviceError::Hid(_) => (5, "usb_or_io"),
        };
        Self {
            code,
            kind,
            message: e.to_string(),
        }
    }
}
impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Self {
            code: 5,
            kind: "io",
            message: e.to_string(),
        }
    }
}
pub(super) fn input(s: impl Into<String>) -> Failure {
    Failure {
        code: 2,
        kind: "invalid_input",
        message: s.into(),
    }
}
pub(super) fn unsupported(s: impl Into<String>) -> Failure {
    Failure {
        code: 6,
        kind: "unsupported",
        message: s.into(),
    }
}

pub fn handles(action: &DeviceAction) -> bool {
    matches!(
        action,
        DeviceAction::Status { .. }
            | DeviceAction::Exec { .. }
            | DeviceAction::ExecFile { .. }
            | DeviceAction::Listener { .. }
            | DeviceAction::Run { remote: true, .. }
    )
}

pub fn open_selected(selected: Option<&str>) -> std::result::Result<Prime, DeviceError> {
    let devices = ppl_device::detect()?;
    if selected.is_none() {
        if devices.len() > 1 {
            return Err(DeviceError::Refused(
                "multiple calculators match; pass --device <serial-id>".into(),
            ));
        }
        return Prime::open(devices.into_iter().next().ok_or(DeviceError::NotFound)?);
    }
    let mut matches = Vec::new();
    for device in devices {
        let mut prime = Prime::open(device)?;
        let info = prime.info()?;
        if info
            .serial
            .as_deref()
            .map(ppl_compat::serial_fingerprint)
            .as_deref()
            == selected
        {
            matches.push(prime);
        }
    }
    if matches.len() > 1 {
        return Err(DeviceError::Refused(
            "multiple calculators share the requested identity".into(),
        ));
    }
    matches
        .pop()
        .ok_or_else(|| DeviceError::Refused("requested device serial-id was not found".into()))
}

fn identity(prime: &mut Prime) -> Result<Identity> {
    let info = prime.info()?;
    let serial = info
        .serial
        .as_ref()
        .ok_or_else(|| unsupported("calculator identity reply has no recognized serial"))?;
    Ok(Identity {
        model: prime.detected.model.into(),
        usb_pid: prime.detected.product_id,
        firmware_build: info.build,
        boot_version: info.version,
        serial_id: ppl_compat::serial_fingerprint(serial),
    })
}
fn reference_g1(id: &Identity) -> Result<()> {
    if id.usb_pid != 0x1541
        || id.firmware_build != Some(15157)
        || id.boot_version.as_deref() != Some("V0.050.640")
    {
        return Err(unsupported(
            "listener experiments are scoped to G1 03f0:1541 build 15157, boot V0.050.640; this firmware is unverified",
        ));
    }
    Ok(())
}
pub(super) fn control() -> ControlObject {
    ControlObject::parse(ppl_device::DEFAULT_CONTROL).expect("constant control")
}
fn load_listener(store: &Store, id: &Identity, prime: &mut Prime) -> Result<Listener> {
    let listener:Listener=store.read(&store.listener_path()).map_err(|e| {
        if e.kind()==std::io::ErrorKind::NotFound { unsupported("no listener installation recorded; use `ppl device listener install`, then start it once on the calculator") }
        else {e.into()}
    })?;
    if listener.schema_version != 1
        || listener.device != *id
        || !listener.uploaded
        || ppl_compat::sha256_hex(listener.source.as_bytes()) != listener.source_sha256
    {
        return Err(unsupported(
            "listener identity/source does not match the installation record",
        ));
    }
    match prime.request_file(FileType::Program, &listener.program)? {
        Probe::Present(f)
            if f.ty == FileType::Program
                && f.name == listener.program
                && f.crc_ok
                && ppl_device::normalize_program_text(&f.text())
                    == ppl_device::normalize_program_text(&listener.source) =>
        {
            Ok(listener)
        }
        _ => Err(unsupported(
            "installed listener source could not be verified; no execution was requested",
        )),
    }
}

fn base(operation: &str) -> Value {
    json!({"schema_version":1,"operation":operation,"device":null,"request_id":null,"uploaded":false,"upload_state":"not_attempted","execution_state":"not_requested","execution_confirmed":false,"result":null,"result_type":null,"error":null,"evidence_paths":[]})
}

pub fn run(opts: &DeviceOpts) -> u8 {
    let operation = match &opts.action {
        DeviceAction::Status { .. } => "status",
        DeviceAction::Exec { .. } => "exec",
        DeviceAction::ExecFile { .. } => "exec_file",
        DeviceAction::Run { .. } => "run",
        DeviceAction::Listener { action } => match action {
            ListenerAction::Install { .. } => "listener_install",
            ListenerAction::FixedMailboxProbe { .. } => "fixed_mailbox_probe",
            ListenerAction::Probe => "listener_probe",
            ListenerAction::Verify => "listener_verify",
            ListenerAction::Status => "listener_status",
            ListenerAction::AbandonProbe { .. } => "listener_abandon_probe",
        },
        _ => unreachable!(),
    };
    let mut output = base(operation);
    let result = perform(opts, &mut output);
    let code = match result {
        Ok(()) => 0,
        Err(e) => {
            output["error"] = json!({"kind":e.kind,"message":e.message,"exit_code":e.code});
            e.code
        }
    };
    if opts.json {
        println!("{output}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&output).expect("JSON value")
        );
    }
    code
}

fn perform(opts: &DeviceOpts, out: &mut Value) -> Result<()> {
    // Validate user-controlled source before opening USB.
    let prepared = match &opts.action {
        DeviceAction::Exec { expression } => {
            super::remote_input::expression(expression).map_err(input)?;
            Some((None, expression.clone()))
        }
        DeviceAction::Run {
            name,
            args,
            remote: true,
            ..
        } => Some((
            None,
            super::remote_input::call(name, args.as_deref()).map_err(input)?,
        )),
        DeviceAction::ExecFile { file, args } => {
            let (source, _) = ppl_syntax::read_source_file(file)?;
            let name = super::remote_input::source(&source.text).map_err(input)?;
            let call = super::remote_input::call(&name, args.as_deref()).map_err(input)?;
            Some((Some((name, source.text)), call))
        }
        DeviceAction::Status {
            request_id: Some(id),
        }
        | DeviceAction::Listener {
            action: ListenerAction::AbandonProbe { request_id: id },
        } => {
            if id.len() != 10 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(input("request ID must contain 10 hex digits"));
            }
            None
        }
        DeviceAction::Listener {
            action: ListenerAction::FixedMailboxProbe { resume: Some(name) },
        } => {
            ppl_device::validate_test_name(name).map_err(|e| input(e.to_string()))?;
            None
        }
        _ => None,
    };
    let mut prime = open_selected(opts.device.as_deref())?;
    let result = (|| {
        let id = identity(&mut prime)?;
        out["device"] = serde_json::to_value(&id).expect("identity serialization");
        prime.log.clear(); // FA contains a raw serial; exclude it from subsequent evidence.
        let store = Store::new(&opts.remote_dir, &id);
        if let DeviceAction::Status { request_id } = &opts.action {
            let deletion = ppl_device::delete::direct_delete_supported(
                id.usb_pid,
                id.firmware_build,
                id.boot_version.as_deref(),
            );
            out["object_management"] = json!({
                "direct_delete": deletion,
                "mechanism": if deletion {Some("usb_delete")} else {None},
                "verification": if deletion {"physically_verified_profile"} else {"unsupported_profile"},
                "key_injection_verified": false,
            });
            if let Some(request_id) = request_id {
                if request_id.len() != 10 || !request_id.bytes().all(|c| c.is_ascii_hexdigit()) {
                    return Err(input("request ID must contain 10 hex digits"));
                }
                let mut journal: Journal = store.read(&store.request_path(request_id))?;
                if journal.device != id {
                    return Err(unsupported(
                        "request belongs to a different device/firmware",
                    ));
                }
                report_journal(&journal, &store, out);
                if matches!(journal.state.as_str(), "submitting" | "abandoned_probe") {
                    let reply = ppl_device::remote::poll(
                        &mut prime,
                        &journal.request,
                        Instant::now() + Duration::from_secs(opts.remote_timeout_secs),
                    )?;
                    journal.reply = Some(reply);
                    journal.state = "completed".into();
                    store.save(&store.request_path(request_id), &journal)?;
                    report_journal(&journal, &store, out);
                }
                return finish_reply(journal.reply.as_ref());
            }
            out["ready_reply"] = json!(prime.check_ready()?);
            out["pending_request"] = json!(store.pending()?.map(|j| j.request.id));
            out["capabilities"] = match load_listener(&store, &id, &mut prime) {
                Ok(l) => json!(l.capabilities),
                Err(e) => {
                    out["capability_note"] = json!(e.message);
                    json!(Capabilities::default())
                }
            };
            out["listener_running"] = Value::Null;
            return Ok(());
        }
        if let DeviceAction::Listener {
            action: ListenerAction::AbandonProbe { request_id },
        } = &opts.action
        {
            if request_id.len() != 10 || !request_id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(input("invalid request ID"));
            }
            let path = store.request_path(request_id);
            let mut journal: Journal = store.read(&path)?;
            if journal.device != id
                || journal.operation != "probe"
                || !journal.request.payload.starts_with("P|")
            {
                return Err(input(
                    "only a harmless echo probe can be abandoned; evaluations remain unresolved",
                ));
            }
            if journal.state != "submitting" {
                return Err(input("probe is not unresolved"));
            }
            journal.state = "abandoned_probe".into();
            store.save(&path, &journal)?;
            report_journal(&journal, &store, out);
            return Ok(());
        }
        reference_g1(&id)?;
        if let DeviceAction::Listener {
            action: ListenerAction::FixedMailboxProbe { resume },
        } = &opts.action
        {
            return super::mailbox_probe::run(&mut prime, &store, opts, out, resume.as_deref());
        }

        if matches!(
            opts.action,
            DeviceAction::Listener {
                action: ListenerAction::Install { .. }
            }
        ) {
            let new_session = matches!(
                opts.action,
                DeviceAction::Listener {
                    action: ListenerAction::Install { new_session: true }
                }
            );
            return install(&mut prime, &store, &id, new_session, out);
        }
        let mut listener = load_listener(&store, &id, &mut prime)?;
        if let DeviceAction::Listener { action } = &opts.action {
            match action {
                ListenerAction::Install { .. }
                | ListenerAction::AbandonProbe { .. }
                | ListenerAction::FixedMailboxProbe { .. } => {
                    unreachable!()
                }
                ListenerAction::Status => {
                    out["capabilities"] = json!(listener.capabilities);
                    out["listener_program"] = json!(listener.program);
                    out["listener_running"] = Value::Null;
                    if let Some(name) = &listener.heartbeat {
                        match prime.probe_file(FileType::Note, name, Duration::from_secs(8))? {
                            Probe::Present(file)
                                if file.name == *name
                                    && file.ty == FileType::Note
                                    && file.crc_ok =>
                            {
                                let text = file.text();
                                if text == "STOPPED" || text.starts_with("ERROR|") {
                                    out["listener_running"] = json!(false);
                                }
                                out["listener_state_note"] = json!(text);
                            }
                            Probe::Present(_) => {
                                return Err(Failure {
                                    code: 9,
                                    kind: "invalid_evidence",
                                    message: "listener state note identity or CRC mismatch".into(),
                                });
                            }
                            Probe::NoReply { .. } => {
                                out["listener_state_note"] = Value::Null;
                            }
                        }
                    }
                    Ok(())
                }
                ListenerAction::Probe => probe(&mut prime, &store, &mut listener, opts, out),
                ListenerAction::Verify => verify(&mut prime, &store, &mut listener, opts, out),
            }
        } else {
            let (file, expression) = prepared.expect("execution request prepared");
            if !listener.capabilities.expressions {
                return Err(unsupported(
                    "physical expression execution is unverified; run `ppl device listener probe` and `listener verify` after starting the listener",
                ));
            }
            if (matches!(opts.action, DeviceAction::Run { .. })
                || super::remote_input::has_unverified_call(&expression))
                && !listener.capabilities.program_calls
            {
                return Err(unsupported("physical program invocation is unverified"));
            }
            if file.is_some() && !listener.capabilities.live_upload {
                return Err(unsupported(
                    "uploading and invoking new programs while the listener runs is unverified",
                ));
            }
            refuse_pending(&store)?;
            if let Some((name, source)) = file {
                out["program"] = json!(name);
                upload_program(&mut prime, &store, &name, &source, out)?;
            }
            transact(&mut prime, &store, &listener, &expression, false, opts, out)?;
            Ok(())
        }
    })();
    if let Some(path) = &opts.log {
        let log = prime.log_text();
        if let Err(e) = std::fs::write(path, log) {
            eprintln!("cannot write requested USB log: {e}");
        }
    }
    result
}

pub(super) fn refuse_pending(store: &Store) -> Result<()> {
    if let Some(journal) = store.pending()? {
        return Err(Failure {
            code: 8,
            kind: "unresolved_request",
            message: format!(
                "request {} may still be running; retrieve it with `ppl device status --request-id {}`; it will not be resubmitted",
                journal.request.id, journal.request.id
            ),
        });
    }
    Ok(())
}
pub(super) fn evidence(out: &mut Value, path: PathBuf) {
    let paths = out["evidence_paths"].as_array_mut().expect("array");
    let value = json!(path);
    if !paths.contains(&value) {
        paths.push(value);
    }
}

fn report_journal(journal: &Journal, store: &Store, out: &mut Value) {
    out["request_id"] = json!(journal.request.id);
    out["request_upload_verified"] = json!(journal.upload_verified);
    out["execution_state"] = json!(if journal.state == "completed" {
        if journal.reply.as_ref().is_some_and(|r| r.success) {
            "completed"
        } else {
            "runtime_error"
        }
    } else if journal.state == "prepared" {
        "not_requested"
    } else {
        "unknown"
    });
    out["execution_confirmed"] =
        json!(journal.operation != "probe" && journal.reply.as_ref().is_some_and(|r| r.success));
    out["result"] = Value::Null;
    out["result_type"] = Value::Null;
    if let Some(reply) = &journal.reply {
        out["result"] = json!(reply.text);
        out["result_type"] = json!(reply.result_type);
    }
    evidence(out, store.request_path(&journal.request.id));
}
fn finish_reply(reply: Option<&ppl_device::remote::Reply>) -> Result<()> {
    if let Some(reply) = reply {
        if !reply.success {
            return Err(Failure {
                code: 3,
                kind: "calculator_error",
                message: reply.text.clone(),
            });
        }
    }
    Ok(())
}

pub(super) fn transact(
    prime: &mut Prime,
    store: &Store,
    listener: &Listener,
    expression: &str,
    probe: bool,
    opts: &DeviceOpts,
    out: &mut Value,
) -> Result<ppl_device::remote::Reply> {
    refuse_pending(store)?;
    let request = ppl_device::remote::Request::new(&listener.session, expression, probe)?;
    let path = store.request_path(&request.id);
    if path.exists() {
        return Err(Failure {
            code: 8,
            kind: "name_collision",
            message: "fresh request ID collided locally; nothing sent".into(),
        });
    }
    let mut journal = Journal {
        schema_version: 1,
        device: listener.device.clone(),
        operation: if probe { "probe" } else { "eval" }.into(),
        request: request.clone(),
        state: "prepared".into(),
        upload_verified: false,
        reply: None,
    };
    store.save(&path, &journal)?;
    report_journal(&journal, store, out);
    eprintln!(
        "preflighting fresh request {} (up to 48 seconds); no execution is confirmed",
        request.id
    );
    let publish = ppl_device::remote::publish(prime, &request, &control(), || {
        journal.state = "submitting".into();
        store
            .save(&path, &journal)
            .map_err(|e| DeviceError::Hid(format!("cannot persist request before upload: {e}")))
    });
    report_journal(&journal, store, out);
    publish?;
    journal.upload_verified = true;
    store.save(&path, &journal)?;
    out["request_upload_verified"] = json!(true);
    eprintln!(
        "request uploaded and read back; waiting for a correlated calculator result ({} seconds)",
        opts.remote_timeout_secs
    );
    let reply = ppl_device::remote::poll(
        prime,
        &request,
        Instant::now() + Duration::from_secs(opts.remote_timeout_secs),
    )?;
    let result_path = store.dir.join(format!("result-{}.txt", request.id));
    std::fs::write(&result_path, &reply.raw_note)?;
    evidence(out, result_path);
    out["result_sha256"] = json!(ppl_compat::sha256_hex(reply.raw_note.as_bytes()));
    journal.reply = Some(reply.clone());
    journal.state = "completed".into();
    store.save(&path, &journal)?;
    report_journal(&journal, store, out);
    finish_reply(Some(&reply))?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_firmware_or_generation_cannot_inherit_execution_support() {
        let reference = Identity {
            model: "test".into(),
            usb_pid: 0x1541,
            firmware_build: Some(15157),
            boot_version: Some("V0.050.640".into()),
            serial_id: "test".into(),
        };
        assert!(reference_g1(&reference).is_ok());
        for candidate in [
            Identity {
                usb_pid: 0x2441,
                ..reference.clone()
            },
            Identity {
                firmware_build: None,
                ..reference.clone()
            },
            Identity {
                firmware_build: Some(15515),
                ..reference.clone()
            },
            Identity {
                boot_version: None,
                ..reference.clone()
            },
        ] {
            assert_eq!(reference_g1(&candidate).unwrap_err().code, 6);
        }
        let capabilities = Capabilities::default();
        assert!(
            !capabilities.mailbox
                && !capabilities.expressions
                && !capabilities.program_calls
                && !capabilities.live_upload
        );
    }
}
