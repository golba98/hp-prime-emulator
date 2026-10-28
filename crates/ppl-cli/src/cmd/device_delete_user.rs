//! Deletion of ordinary programs and notes (`class: "user"`).
//!
//! Unlike disposable test objects, an ordinary object is never authorized by
//! a creation receipt. Authorization is either the exact typed name
//! (interactive) or the explicit `--yes` flag, bound to this device, type,
//! name and content. Single target, raw backup before anything is sent,
//! single `EB`, mandatory independent absence verification, no helper.
use super::*;
use ppl_device::delete::Baseline;

/// How the deletion was authorized.
pub(super) enum Authorization {
    /// Type the exact name; refused when the session is not interactive.
    Typed { interactive: bool },
    /// `--yes`: no stdin, scoped to the one named object on this device.
    Yes,
}

/// Prints human progress lines; `[OK]` is written only after a step has
/// actually succeeded. Disabled (no output) in JSON mode.
pub(super) struct Progress<'a>(pub Option<&'a mut dyn std::io::Write>);
impl Progress<'_> {
    fn line(&mut self, text: &str) {
        if let Some(w) = self.0.as_mut() {
            let _ = writeln!(w, "{text}");
        }
    }
    fn ok(&mut self, text: &str) {
        self.line(&format!("[OK] {text}"));
    }
}

/// What the user is shown before authorizing a deletion.
pub(super) struct Summary<'a> {
    pub serial: &'a str,
    pub ty: FileType,
    pub name: &'a str,
    pub bytes: usize,
    pub sha256: &'a str,
    pub backup: &'a Path,
}

/// Obtains explicit, target-specific authorization. Refuses without any
/// prompt when the session is not interactive (including `--json`). The
/// typed name must equal the object name exactly: no case folding, trimming
/// or abbreviation (only the line terminator is removed).
pub(super) fn authorize_ordinary(
    input: &mut impl std::io::BufRead,
    out: &mut impl std::io::Write,
    interactive: bool,
    s: &Summary<'_>,
) -> Result<&'static str, DeviceError> {
    if !interactive {
        return Err(refused(format!(
            "deleting `{}` requires interactive confirmation (type the exact name); \
             non-interactive and --json sessions cannot authorize deletion of ordinary objects",
            s.name
        )));
    }
    let label = s.ty.label();
    writeln!(
        out,
        "HP Prime G1 — Delete {label}\n\n\
         Device: {}\nObject type: {label}\nObject name: {}\n\n\
         The {label} currently exists. Its contents ({} bytes, sha256 {}) were read twice and verified.\n\
         A raw backup was saved and synced before this prompt:\n  {}\n",
        s.serial,
        s.name,
        s.bytes,
        s.sha256,
        s.backup.display()
    )
    .and_then(|_| write!(out, "Type {} to authorize deletion: ", s.name))
    .and_then(|_| out.flush())
    .map_err(disk)?;
    let mut line = String::new();
    input.read_line(&mut line).map_err(disk)?;
    if line.trim_end_matches(['\r', '\n']) != s.name {
        return Err(refused(
            "typed authorization did not match; nothing was sent",
        ));
    }
    Ok("exact_typed_confirmation")
}

fn result_of(op: &Operation) -> &'static str {
    let any = |s: &str| op.targets.iter().any(|t| t.status == s);
    if op.confirmed() {
        "confirmed_deleted"
    } else if any("still_present") {
        "still_present"
    } else if op
        .error
        .as_deref()
        .is_some_and(|e| e.contains("did not answer"))
    {
        "device_unresponsive"
    } else {
        "indeterminate"
    }
}

/// Maps a failure that happened before the device was mutated.
pub(super) fn failure_result(e: &DeviceError) -> &'static str {
    match e {
        DeviceError::Unsupported(_) => "unsupported_object_type",
        DeviceError::Absent(_) => "not_found",
        DeviceError::Refused(m) if m.starts_with("unsupported device") => "unsupported_device",
        DeviceError::Inconclusive(m) if m.contains("did not answer") => "device_unresponsive",
        DeviceError::NotFound | DeviceError::Timeout(_) | DeviceError::Hid(_) => {
            "device_unresponsive"
        }
        _ => "refused",
    }
}

fn finish(
    store: &Store,
    op: &Operation,
    result: &str,
    backup_created: bool,
    code: u8,
) -> (Value, u8) {
    let mut v = outcome(store, op);
    v["result"] = json!(result);
    v["object_class"] = json!("user");
    v["authorization"] = json!(op.authorization);
    v["backup_created"] = json!(backup_created);
    v["status"] = json!(result);
    v["object"] = json!(op.targets[0].name);
    v["type"] = json!(op.targets[0].ty);
    v["firmware_build"] = json!(op.firmware_build);
    v["liveness_anchor"] = json!(op.control);
    v["backup_sha256"] = json!(op.targets[0].sha256);
    v["backup_path"] = json!(
        store
            .directory(&op.id)
            .ok()
            .map(|d| d.join(format!("{}-{}.bin", op.targets[0].ty, op.targets[0].name)))
    );
    v["submission_state"] = json!(
        if matches!(op.state.as_str(), "prepared" | "not_submitted") {
            "not_sent"
        } else if op
            .submission
            .contains(&ppl_device::delete::SubmissionState::PossiblySent)
        {
            "possibly_sent"
        } else if op
            .submission
            .contains(&ppl_device::delete::SubmissionState::Sent)
        {
            "submitted"
        } else if op.state == "submitting" {
            "sending"
        } else {
            "unknown"
        }
    );
    v["verification"] = json!(if op.confirmed() {
        "absence_confirmed"
    } else {
        "not_confirmed"
    });
    (v, code)
}

/// Picks the F8 liveness anchor: an explicit `--control`, else the built-in
/// settings object (only when its use is evidenced for this profile), else
/// an already-installed default control. Silence from any other source
/// proves nothing, so with no anchor the deletion is refused up front.
fn choose_anchor(
    p: &mut Prime,
    explicit: Option<&ControlObject>,
    settings_verified: bool,
) -> Result<ControlObject, DeviceError> {
    if let Some(c) = explicit {
        return Ok(c.clone());
    }
    let mut candidates = Vec::new();
    if settings_verified {
        candidates
            .push(ControlObject::parse(ppl_device::DEFAULT_SETTINGS_ANCHOR).map_err(refused)?);
    }
    candidates.push(ControlObject::parse(ppl_device::DEFAULT_CONTROL).map_err(refused)?);
    for c in candidates {
        match p.probe_file(c.ty, &c.name, ppl_device::PROBE_TIMEOUT)? {
            Probe::Present(f) if f.name == c.name && f.ty == c.ty && f.crc_ok => return Ok(c),
            Probe::Present(f) => {
                return Err(DeviceError::Protocol(format!(
                    "anchor probe for `{}` was answered with `{}`",
                    c.name, f.name
                )));
            }
            Probe::NoReply { .. } => {}
        }
    }
    Err(refused(
        "unsupported device: no liveness anchor answered. Without a file the calculator is known \
         to serve, silence after a delete could not be told from a stalled calculator, so nothing \
         was sent. Install a control program or pass --control.",
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_user(
    targets: Vec<DeleteTarget>,
    explicit_control: Option<ControlObject>,
    auth_mode: AuthMode,
    runs_dir: &Path,
    delete_dir: &Path,
    json_output: bool,
    selected: Option<&str>,
    log_path: Option<&Path>,
) -> Result<(Value, u8), DeviceError> {
    let mut p = crate::cmd::remote::open_selected(selected)?;
    let (serial, info) = identity(&mut p)?;
    let auth = match auth_mode {
        AuthMode::Yes => Authorization::Yes,
        AuthMode::Typed => Authorization::Typed {
            interactive: !json_output && std::io::stdin().is_terminal(),
        },
    };
    let mut stdout = std::io::stdout();
    let mut progress = Progress(if json_output { None } else { Some(&mut stdout) });
    perform_user(
        &mut p,
        &serial,
        &info,
        &targets,
        explicit_control.as_ref(),
        ppl_device::SETTINGS_ANCHOR_VERIFIED,
        auth,
        (runs_dir, delete_dir, log_path),
        &mut std::io::stdin().lock(),
        &mut std::io::stderr(),
        &mut progress,
    )
}

/// Which authorization the command line asked for.
#[derive(Clone, Copy)]
pub(super) enum AuthMode {
    Typed,
    Yes,
}

/// Everything after the device has been opened and identified. Input and
/// output are injected so the whole flow can run against a mock device.
#[allow(clippy::too_many_arguments)]
pub(super) fn perform_user(
    p: &mut Prime,
    serial: &str,
    info: &ppl_device::CalcInfo,
    targets: &[DeleteTarget],
    explicit_control: Option<&ControlObject>,
    settings_verified: bool,
    auth: Authorization,
    (runs_dir, delete_dir, log_path): (&Path, &Path, Option<&Path>),
    input: &mut impl std::io::BufRead,
    prompt_out: &mut impl std::io::Write,
    progress: &mut Progress<'_>,
) -> Result<(Value, u8), DeviceError> {
    if !direct_profile(p, info) {
        return Err(refused(
            "unsupported device: deleting ordinary objects needs the hardware-verified USB delete \
             profile (G1 03f0:1541, build 15157)",
        ));
    }
    let target = &targets[0];
    progress.line(&format!(
        "HP Prime G1\nFirmware: build {}\n\nTarget: {} {}\n",
        info.build.map_or("unknown".into(), |b| b.to_string()),
        target.ty.label(),
        target.name
    ));
    progress.ok("Device connected");
    let baseline = Baseline::from_info(info)?;
    let control = choose_anchor(p, explicit_control, settings_verified)?;
    progress.ok(&format!("Liveness anchor verified ({control})"));
    let store = Store::new(delete_dir);
    let _journal_lock = store.lock()?;
    store.reject_pending(serial, targets)?;
    let deletion = ppl_device::prepare_deletion_with(
        p,
        targets,
        &control,
        ppl_device::NamePolicy::User,
        Some(&baseline),
    )?;
    progress.ok(&format!("{} found", capitalize(target.ty.label())));
    progress.ok(&format!(
        "{} contents verified (two reads, CRC valid)",
        capitalize(target.ty.label())
    ));
    let now = ppl_compat::utc_now();
    let mut op = Operation {
        schema_version: 1,
        id: deletion.helper.clone(),
        device_serial_id: serial.into(),
        usb_pid: p.detected.product_id,
        firmware_build: info.build,
        boot_version: info.version.clone(),
        control: control.to_string(),
        state: "prepared".into(),
        mechanism: "usb_delete".into(),
        created_utc: now.clone(),
        updated_utc: now.clone(),
        targets: deletion
            .backups
            .iter()
            .map(|b| Target {
                name: b.name.clone(),
                ty: b.ty.label().into(),
                sha256: ppl_compat::sha256_hex(&b.data),
                bytes: b.data.len(),
                status: "not_submitted".into(),
            })
            .collect(),
        helper: None,
        helper_note: None,
        helper_source: None,
        helper_uploaded: false,
        completion: None,
        artifacts: Vec::new(),
        cleanup_state: "not_required".into(),
        runs_dir: runs_dir.to_path_buf(),
        error: None,
        class: "user".into(),
        authorization: None,
        requested_utc: Some(now.clone()),
        submission: Vec::new(),
    };
    let b = &deletion.backups[0];
    // 1. Persist and re-read the raw backup (plus metadata) before anything
    //    is authorized or sent.
    let backup_ok = store
        .create(&op, &deletion.backups)
        .and_then(|_| {
            let dir = store.directory(&op.id)?;
            records::atomic_json(
                &dir.join("metadata.json"),
                &json!({
                    "schema_version": 1,
                    "object": {"type": b.ty.label(), "name": b.name,
                               "bytes": b.data.len(), "sha256": op.targets[0].sha256},
                    "device": {"fingerprint": serial, "usb_pid": op.usb_pid,
                               "firmware_build": op.firmware_build,
                               "boot_version": op.boot_version},
                    "operation_id": op.id,
                    "backup_file": format!("{}-{}.bin", b.ty.label(), b.name),
                    "format": "raw object bytes as sent by the calculator (UTF-16LE text)",
                    "created_utc": now,
                    "restorable": "not verified",
                }),
            )
            .map_err(disk)
        })
        .and_then(|_| {
            let again = store.backups(&op)?;
            if again.len() == deletion.backups.len()
                && again
                    .iter()
                    .zip(&deletion.backups)
                    .all(|(a, b)| a.data == b.data && a.name == b.name && a.ty == b.ty)
            {
                Ok(())
            } else {
                Err(DeviceError::Refused(
                    "saved backup does not match the object that was read".into(),
                ))
            }
        });
    if let Err(e) = backup_ok {
        op.error = Some(format!("backup failed; nothing was sent: {e}"));
        op.state = "not_submitted".into();
        let _ = store.save(&op);
        return Ok(finish(&store, &op, "backup_failed", false, exit::IO));
    }
    progress.ok(&format!(
        "Local backup created ({})",
        store.directory(&op.id)?.display()
    ));
    let backup_path = store
        .directory(&op.id)?
        .join(format!("{}-{}.bin", b.ty.label(), b.name));
    // 2. Authorization bound to this device, type, name and content.
    let method = match auth {
        Authorization::Yes => Ok("explicit_yes_flag"),
        Authorization::Typed { interactive } => authorize_ordinary(
            input,
            prompt_out,
            interactive,
            &Summary {
                serial,
                ty: b.ty,
                name: &b.name,
                bytes: b.data.len(),
                sha256: &op.targets[0].sha256,
                backup: &backup_path,
            },
        ),
    };
    let method = match method {
        Ok(m) => m,
        Err(e) => {
            op.error = Some(e.to_string());
            op.state = "not_submitted".into();
            let _ = store.save(&op);
            return Ok(finish(&store, &op, "authorization_refused", true, exit::IO));
        }
    };
    op.authorization = Some(method.into());
    // 3. Record the authorization and the submission intent durably.
    let recorded = records::atomic_json(
        &store.directory(&op.id)?.join("authorization.json"),
        &json!({
            "device_serial_id": op.device_serial_id,
            "class": "user",
            "authorization": method,
            "target": {"type": b.ty.label(), "name": b.name, "sha256": op.targets[0].sha256},
            "requested_utc": op.requested_utc,
            "scope": "one deletion attempt of exactly this object on this device",
        }),
    )
    .map_err(disk)
    .and_then(|_| {
        op.state = "submitting".into();
        store.save(&op)
    });
    if let Err(e) = recorded {
        op.error = Some(format!(
            "cannot record authorization; nothing was sent: {e}"
        ));
        op.state = "not_submitted".into();
        let _ = store.save(&op);
        return Ok(finish(&store, &op, "backup_failed", true, exit::IO));
    }
    progress.ok("Deletion authorized");
    // 4. One EB, then independent absence verification. An Err from here
    // means nothing was transmitted (all sends happen inside Ok).
    let result = ppl_programs::submit_deletion(p, &deletion, &store, &mut op);
    match result {
        Ok(report) => {
            if report
                .submission
                .contains(&ppl_device::delete::SubmissionState::Sent)
            {
                progress.ok("Native USB delete submitted");
            }
            let never_sent = report
                .submission
                .iter()
                .all(|s| *s == ppl_device::delete::SubmissionState::NotSent);
            op.submission = report.submission;
            apply_statuses(
                &mut op,
                &report
                    .targets
                    .iter()
                    .map(|t| t.status.clone())
                    .collect::<Vec<_>>(),
            );
            op.error = report.error;
            op.state = if never_sent {
                "not_submitted"
            } else if op.confirmed() {
                "completed"
            } else {
                "unresolved"
            }
            .into();
            if op.confirmed() {
                progress.ok("Independent absence verified");
            }
        }
        Err(e) => {
            op.error = Some(format!("nothing was sent: {e}"));
            op.state = "not_submitted".into();
        }
    }
    op.updated_utc = ppl_compat::utc_now();
    if let Err(e) = store.save(&op) {
        op.error = Some(format!(
            "{}; {e}",
            op.error
                .as_deref()
                .unwrap_or("device outcome recorded in memory")
        ));
    }
    if let Some(path) = log_path {
        if let Err(e) = std::fs::write(path, p.log_text()) {
            op.error = Some(format!("cannot save USB log: {e}"));
        }
    }
    let result = if op.state == "not_submitted" {
        op.error
            .as_deref()
            .filter(|e| e.contains("did not answer"))
            .map_or("refused", |_| "device_unresponsive")
    } else {
        result_of(&op)
    };
    let code = if op.confirmed() && op.error.is_none() {
        progress.line(&format!(
            "\nResult: {} deleted successfully.",
            op.targets[0].name
        ));
        exit::OK
    } else {
        exit::IO
    };
    Ok(finish(&store, &op, result, true, code))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppl_device::NamePolicy;

    fn summary<'a>(name: &'a str, backup: &'a Path) -> Summary<'a> {
        Summary {
            serial: "0123456789abcdef",
            ty: FileType::Program,
            name,
            bytes: 12,
            sha256: "ab",
            backup,
        }
    }
    fn ask(typed: &str, interactive: bool) -> (Result<&'static str, DeviceError>, String) {
        let mut out = Vec::new();
        let r = authorize_ordinary(
            &mut typed.as_bytes(),
            &mut out,
            interactive,
            &summary("FIREWORKS", Path::new("/b/program-FIREWORKS.bin")),
        );
        (r, String::from_utf8(out).unwrap())
    }
    fn control() -> ControlObject {
        ControlObject::parse(ppl_device::DEFAULT_CONTROL).unwrap()
    }
    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn exact_name_is_the_only_accepted_confirmation() {
        let (ok, shown) = ask("FIREWORKS\n", true);
        assert_eq!(ok.unwrap(), "exact_typed_confirmation");
        assert!(shown.contains("Delete program") && shown.contains("0123456789abcdef"));
        assert!(shown.contains("backup was saved") && shown.contains("program-FIREWORKS.bin"));
        assert!(ask("FIREWORKS\r\n", true).0.is_ok());
        for wrong in [
            "fireworks\n",
            "FIREWORK\n",
            "FIREWORKS \n",
            " FIREWORKS\n",
            "\n",
            "",
            "y\n",
        ] {
            assert!(
                ask(wrong, true).0.is_err(),
                "`{wrong:?}` must not authorize"
            );
        }
    }

    #[test]
    fn non_interactive_sessions_never_prompt_or_authorize() {
        let (r, shown) = ask("FIREWORKS\n", false);
        assert!(r.is_err());
        assert!(shown.is_empty(), "no prompt may be shown");
    }

    #[test]
    fn classification_separates_test_and_ordinary_objects() {
        let c = control();
        let (p, t) = classify(FileType::Program, &names(&["FIREWORKS"]), &c, false).unwrap();
        assert_eq!((p, t[0].name.as_str()), (NamePolicy::User, "FIREWORKS"));
        let (p, _) = classify(FileType::Note, &names(&["MyNote"]), &c, false).unwrap();
        assert_eq!(p, NamePolicy::User);
        let (p, t) = classify(FileType::Program, &names(&["ZZT_A", "ZZT_B"]), &c, false).unwrap();
        assert_eq!((p, t.len()), (NamePolicy::Test, 2));
        assert_eq!(
            classify(FileType::Program, &names(&["ZZT_TEST"]), &c, false)
                .unwrap()
                .0,
            NamePolicy::Test
        );
        for bad in [
            names(&["FIREWORKS", "HELLO"]),
            names(&["FIREWORKS", "ZZT_A"]),
            names(&["../x"]),
            names(&["A B"]),
            names(&[]),
            names(&["zzt_ctrl"]),
        ] {
            assert!(
                classify(FileType::Program, &bad, &c, false).is_err(),
                "{bad:?}"
            );
        }
        assert!(classify(FileType::Program, &names(&["ZZT_CTRL"]), &c, false).is_err());
        assert!(matches!(
            classify(FileType::List, &names(&["FIREWORKS"]), &c, false),
            Err(DeviceError::Unsupported(_))
        ));
    }

    #[test]
    fn failures_map_to_the_documented_result_vocabulary() {
        assert_eq!(
            failure_result(&DeviceError::Unsupported("list".into())),
            "unsupported_object_type"
        );
        assert_eq!(
            failure_result(&DeviceError::Inconclusive(
                "control `X` did not answer".into()
            )),
            "device_unresponsive"
        );
        assert_eq!(
            failure_result(&DeviceError::NotFound),
            "device_unresponsive"
        );
    }

    fn user_op(dir: &Path) -> (Store, Operation, ppl_device::Backup) {
        let b = ppl_device::Backup {
            ty: FileType::Program,
            name: "FIREWORKS".into(),
            data: b"F\0I\0".to_vec(),
            crc_ok: true,
        };
        let op = Operation {
            schema_version: 1,
            id: "ZZT_D12345".into(),
            device_serial_id: "0123456789abcdef".into(),
            usb_pid: 0x1541,
            firmware_build: Some(15157),
            boot_version: Some("V0.050.640".into()),
            control: ppl_device::DEFAULT_CONTROL.into(),
            state: "prepared".into(),
            mechanism: "usb_delete".into(),
            created_utc: "t".into(),
            updated_utc: "t".into(),
            targets: vec![Target {
                name: b.name.clone(),
                ty: "program".into(),
                sha256: ppl_compat::sha256_hex(&b.data),
                bytes: b.data.len(),
                status: "not_submitted".into(),
            }],
            helper: None,
            helper_note: None,
            helper_source: None,
            helper_uploaded: false,
            completion: None,
            artifacts: Vec::new(),
            cleanup_state: "not_required".into(),
            runs_dir: dir.join("runs"),
            error: None,
            class: "user".into(),
            authorization: None,
            requested_utc: None,
            submission: Vec::new(),
        };
        (Store::new(&dir.join("store")), op, b)
    }
    fn tmp() -> PathBuf {
        std::env::temp_dir().join(ppl_device::fresh_test_name())
    }

    #[test]
    fn user_records_roundtrip_but_never_touch_ownership_receipts() {
        let dir = tmp();
        let (store, op, b) = user_op(&dir);
        store.create(&op, std::slice::from_ref(&b)).unwrap();
        let loaded = store.load(&op.id).unwrap();
        assert_eq!(loaded.class, "user");
        assert_eq!(store.backups(&loaded).unwrap()[0].data, b.data);
        // No receipt exists, and retiring one is a no-op for user objects.
        assert!(records::owns(&loaded.runs_dir, "0123456789abcdef", &b).is_err());
        assert!(!dir.join("runs").exists());
        // A different device or type is not blocked by this operation.
        let t = [DeleteTarget::program("FIREWORKS")];
        assert!(store.reject_pending("0123456789abcdef", &t).is_err());
        assert!(store.reject_pending("1123456789abcdef", &t).is_ok());
        assert!(
            store
                .reject_pending("0123456789abcdef", &[DeleteTarget::note("FIREWORKS")])
                .is_ok()
        );
    }

    #[test]
    fn corrupt_missing_or_inconsistent_user_records_are_refused() {
        let dir = tmp();
        let (store, mut op, b) = user_op(&dir);
        store.create(&op, std::slice::from_ref(&b)).unwrap();
        let bin = store
            .directory(&op.id)
            .unwrap()
            .join("program-FIREWORKS.bin");
        std::fs::write(&bin, b"tampered").unwrap();
        assert!(store.backups(&op).is_err());
        std::fs::remove_file(&bin).unwrap();
        assert!(store.backups(&op).is_err());
        // A user record cannot smuggle in a helper or a second target.
        op.helper = Some(op.id.clone());
        store.save(&op).unwrap();
        assert!(store.load(&op.id).is_err());
    }

    #[test]
    fn legacy_records_without_class_load_as_test_records() {
        let dir = tmp();
        let (store, mut op, _) = user_op(&dir);
        op.class = "test".into();
        op.targets[0].name = "ZZT_TEST".into();
        store.create(&op, &[]).unwrap();
        let path = store.directory(&op.id).unwrap().join("operation.json");
        let mut v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let o = v.as_object_mut().unwrap();
        for k in ["class", "authorization", "requested_utc"] {
            o.remove(k);
        }
        std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        let loaded = store.load(&op.id).unwrap();
        assert_eq!(loaded.policy(), NamePolicy::Test);
        // ...and an ordinary name is not acceptable in a test-class record.
        v["targets"][0]["name"] = json!("FIREWORKS");
        std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(store.load(&op.id).is_err());
    }
}
