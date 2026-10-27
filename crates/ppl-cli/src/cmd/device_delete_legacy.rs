//! Explicit migration of previously confirmed manual-deletion evidence.
//! A prefix or a `done` note alone never establishes ownership.
use super::device_delete_store::{Artifact, Operation, Store, Target};
use ppl_device::{Backup, ControlObject, DeviceError, FileType, Prime, Probe, TargetStatus};
use serde_json::Value;
use std::path::Path;

fn refuse(s: &str) -> DeviceError {
    DeviceError::Refused(format!("legacy deletion evidence: {s}"))
}

/// Import only an explicitly named historical helper, with matching device,
/// immutable target backups, a terminal verification event and live evidence.
pub fn import(
    store: &Store,
    id: &str,
    p: &mut Prime,
    serial: &str,
    info: &ppl_device::CalcInfo,
) -> Result<Operation, DeviceError> {
    ppl_device::validate_test_name(id)?;
    if !id.starts_with("ZZT_D") {
        return Err(refuse("not a generated deletion helper"));
    }
    let journal = std::fs::read_to_string(store.root.join("journal.jsonl"))
        .map_err(|e| refuse(&e.to_string()))?;
    let events: Vec<Value> = journal
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .map_err(|_| refuse("journal contains a malformed event"))?;
    let events: Vec<_> = events
        .iter()
        .filter(|e| e["id"].as_str() == Some(id))
        .collect();
    let prepared = events
        .iter()
        .find(|e| {
            e["phase"] == "prepared"
                && e["targets"]
                    .as_array()
                    .is_some_and(|t| t.iter().all(|t| t["backup_sha256"].is_string()))
        })
        .ok_or_else(|| refuse("no original prepared record"))?;
    let old_id = info.serial_fingerprint();
    let recorded = prepared["device_serial_id"]
        .as_str()
        .ok_or_else(|| refuse("missing device identity"))?;
    if recorded != serial && Some(recorded) != old_id.as_deref() {
        return Err(refuse("prepared record belongs to another device"));
    }
    let terminal = events
        .iter()
        .rev()
        .find(|e| e["phase"] == "verify")
        .ok_or_else(|| refuse("no independent terminal verification"))?;
    if ![Some(serial), old_id.as_deref()].contains(&terminal["device_serial_id"].as_str()) {
        return Err(refuse("verification belongs to another device"));
    }
    let terminal_targets = terminal["targets"]
        .as_array()
        .ok_or_else(|| refuse("missing verification targets"))?;
    let mut backups = Vec::new();
    let mut targets = Vec::new();
    for b in prepared["targets"]
        .as_array()
        .ok_or_else(|| refuse("missing prepared targets"))?
    {
        let name = b["name"]
            .as_str()
            .ok_or_else(|| refuse("invalid target name"))?;
        ppl_device::validate_test_name(name)?;
        let ty = b["type"]
            .as_str()
            .and_then(FileType::parse)
            .filter(|t| matches!(t, FileType::Program | FileType::Note))
            .ok_or_else(|| refuse("invalid target type"))?;
        if !terminal_targets.iter().any(|t| {
            t["object"].as_str() == Some(name)
                && t["type"].as_str() == Some(ty.label())
                && t["deletion_confirmed"] == true
        }) {
            return Err(refuse("target lacks confirmed deletion"));
        }
        let ext = if ty == FileType::Program {
            "hpprgm"
        } else {
            "txt"
        };
        let data = std::fs::read(store.root.join("backups").join(format!("{name}.{ext}")))
            .map_err(|_| refuse("original backup missing"))?;
        if b["backup_sha256"].as_str() != Some(ppl_compat::sha256_hex(&data).as_str()) {
            return Err(refuse("original backup hash mismatch"));
        }
        targets.push(Target {
            name: name.into(),
            ty: ty.label().into(),
            sha256: ppl_compat::sha256_hex(&data),
            bytes: data.len(),
            status: "confirmed_deleted".into(),
        });
        backups.push(Backup {
            ty,
            name: name.into(),
            data,
            crc_ok: true,
        });
    }
    let control =
        ControlObject::parse(ppl_device::DEFAULT_CONTROL).map_err(|_| refuse("invalid control"))?;
    if targets.is_empty()
        || targets
            .iter()
            .any(|t| t.name.eq_ignore_ascii_case(&control.name))
    {
        return Err(refuse("invalid target set"));
    }
    let requested = targets
        .iter()
        .map(|t| t.target(ppl_device::NamePolicy::Test))
        .collect::<Result<Vec<_>, _>>()?;
    if ppl_device::verify_deleted(p, &requested, &control)?
        .iter()
        .any(|s| *s != TargetStatus::ConfirmedDeleted)
    {
        return Err(refuse("original target no longer absent"));
    }
    // Unknown historical helper source is never guessed or deleted.
    if ppl_device::verify_deleted(p, &[ppl_device::DeleteTarget::program(id)], &control)?
        != [TargetStatus::ConfirmedDeleted]
    {
        return Err(refuse(
            "historical helper still present; source is not archived",
        ));
    }
    let note = ppl_device::result_note_name(id);
    let file = match p.request_file(FileType::Note, &note)? {
        Probe::Present(f)
            if f.ty == FileType::Note
                && f.name == note
                && f.crc_ok
                && ppl_device::normalize_program_text(&f.text()) == "done" =>
        {
            f
        }
        _ => return Err(refuse("terminal note is missing, changed or corrupt")),
    };
    let now = ppl_compat::utc_now();
    let op = Operation {
        schema_version: 1,
        id: id.into(),
        device_serial_id: serial.into(),
        usb_pid: p.detected.product_id,
        firmware_build: info.build,
        boot_version: info.version.clone(),
        control: control.to_string(),
        state: "verified".into(),
        mechanism: "ppl_helper_legacy".into(),
        created_utc: now.clone(),
        updated_utc: now,
        targets,
        helper: Some(id.into()),
        helper_note: Some(note.clone()),
        helper_source: None,
        helper_uploaded: true,
        completion: Some("done".into()),
        artifacts: vec![Artifact {
            name: note,
            ty: "note".into(),
            sha256: ppl_compat::sha256_hex(&file.data),
            text: file.text(),
            status: "present".into(),
        }],
        cleanup_state: "pending".into(),
        runs_dir: Path::new(ppl_compat::DEFAULT_RUNS_DIR).into(),
        error: None,
        class: "test".into(),
        authorization: None,
        requested_utc: None,
        submission: Vec::new(),
    };
    store.create(&op, &backups)?;
    Ok(op)
}
