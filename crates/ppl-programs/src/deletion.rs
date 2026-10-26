//! Explicit single-program deletion; uses the same guarded EB implementation as
//! the CLI. Preparation is read-only; an operation ID is never a target name.
use crate::{
    DeviceIdentity, Result,
    delete_store::{Operation, Store, Target, atomic_json},
    disk, refused,
};
use ppl_device::{
    ControlObject, DeleteTarget, NamePolicy, Prime, TargetStatus,
    delete::{Baseline, SubmissionState},
};

pub fn prepare(prime: &mut Prime, name: &str, store: &Store) -> Result<Operation> {
    ppl_device::validate_program_name(name)?;
    let _lock = store.lock()?;
    let device = DeviceIdentity::read(prime)?;
    device.require_reference()?;
    store.reject_pending(&device.serial_id, &[DeleteTarget::program(name)])?;
    let control = ControlObject::parse(ppl_device::DEFAULT_SETTINGS_ANCHOR).map_err(refused)?;
    let baseline = Baseline::from_info(&prime.info()?)?;
    let d = ppl_device::prepare_deletion_with(
        prime,
        &[DeleteTarget::program(name)],
        &control,
        NamePolicy::User,
        Some(&baseline),
    )?;
    let now = ppl_compat::utc_now();
    let op = Operation {
        schema_version: 1,
        id: d.helper.clone(),
        device_serial_id: device.serial_id,
        usb_pid: device.usb_pid,
        firmware_build: device.firmware_build,
        boot_version: device.boot_version,
        control: control.to_string(),
        state: "prepared".into(),
        mechanism: "usb_delete".into(),
        created_utc: now.clone(),
        updated_utc: now,
        targets: d
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
        artifacts: vec![],
        cleanup_state: "not_required".into(),
        runs_dir: store.root.clone(),
        error: None,
        class: "user".into(),
        authorization: None,
        requested_utc: None,
        submission: vec![],
    };
    store.create(&op, &d.backups)?;
    store.backups(&op)?;
    Ok(op)
}
pub fn apply(prime: &mut Prime, store: &Store, id: &str, old_sha256: &str) -> Result<Operation> {
    let _lock = store.lock()?;
    let mut op = store.load(id)?;
    store.reject_other_pending(&op.device_serial_id, &op.targets()?, Some(id))?;
    if op.targets.len() != 1 || op.targets[0].ty != "program" {
        return Err(refused("management apply requires one program target"));
    }
    if store.directory(id)?.join("replacement.json").exists() {
        return Err(refused(
            "this operation is a replacement, not a standalone deletion",
        ));
    }
    if op.state != "prepared"
        || op.authorization.is_some()
        || op.targets.len() != 1
        || op.targets[0].sha256 != old_sha256
    {
        return Err(refused(
            "deletion authorization does not match an unattempted prepared operation",
        ));
    }
    let device = DeviceIdentity {
        serial_id: op.device_serial_id.clone(),
        usb_pid: op.usb_pid,
        firmware_build: op.firmware_build,
        boot_version: op.boot_version.clone(),
    };
    device.require_reference()?;
    device.verify(prime)?;
    let backups = store.backups(&op)?;
    let base = Baseline::from_info(&prime.info()?)?;
    let d = ppl_device::prepare_deletion_with(
        prime,
        &op.targets()?,
        &op.control()?,
        NamePolicy::User,
        Some(&base),
    )?;
    if d.backups[0].data != backups[0].data {
        return Err(refused("program changed since preparation"));
    }
    atomic_json(&store.directory(id)?.join("authorization.json"),&serde_json::json!({"operation_id":id,"device":device,"target":op.targets[0],"scope":"one explicit deletion; no automatic retry"})).map_err(disk)?;
    op.authorization = Some("explicit_content_bound_plan".into());
    op.state = "submitting".into();
    store.save(&op)?;
    match crate::submit_deletion(prime, &d, store, &mut op) {
        Ok(report) => {
            op.submission = report.submission;
            op.error = report.error;
            for (t, r) in op.targets.iter_mut().zip(report.targets) {
                t.status = match r.status {
                    TargetStatus::ConfirmedDeleted => "confirmed_deleted",
                    TargetStatus::StillPresent => "still_present",
                    TargetStatus::Unknown => "unknown",
                }
                .into();
            }
            op.state = if op.submission.iter().all(|s| *s == SubmissionState::NotSent) {
                "not_submitted"
            } else if op.confirmed() {
                "completed"
            } else {
                "unresolved"
            }
            .into();
        }
        Err(error) => {
            op.state = "not_submitted".into();
            op.error = Some(error.to_string());
        }
    }
    op.updated_utc = ppl_compat::utc_now();
    store.save(&op)?;
    Ok(op)
}

/// Caller holds the journal lock; this performs only identity/anchor/F8 reads.
pub fn reconcile(prime: &mut Prime, store: &Store, op: &mut Operation) -> Result<()> {
    if store.directory(&op.id)?.join("replacement.json").exists() {
        return Err(refused("use replacement recovery for this operation"));
    }
    let device = DeviceIdentity {
        serial_id: op.device_serial_id.clone(),
        usb_pid: op.usb_pid,
        firmware_build: op.firmware_build,
        boot_version: op.boot_version.clone(),
    };
    device.verify(prime)?;
    store.backups(op)?;
    let base = Baseline::from_info(&prime.info()?)?;
    let statuses = ppl_device::delete::verify_deleted_with(
        prime,
        &op.targets()?,
        &op.control()?,
        Some(&base),
    )?;
    for (target, status) in op.targets.iter_mut().zip(statuses) {
        target.status = match status {
            TargetStatus::ConfirmedDeleted => "confirmed_deleted",
            TargetStatus::StillPresent => "still_present",
            TargetStatus::Unknown => "unknown",
        }
        .into();
    }
    let known_not_sent =
        !op.submission.is_empty() && op.submission.iter().all(|s| *s == SubmissionState::NotSent);
    let legacy_not_sent = op.targets.iter().all(|t| t.status == "still_present")
        && crate::legacy_no_send::resolve(prime, store, op)?;
    if legacy_not_sent {
        op.submission = vec![SubmissionState::NotSent];
    }
    op.error = None;
    op.state = if known_not_sent || legacy_not_sent {
        "not_submitted"
    } else if op.confirmed() {
        if op.helper.is_some() && op.cleanup_state != "complete" {
            "verified"
        } else {
            "completed"
        }
    } else {
        "unresolved"
    }
    .into();
    op.updated_utc = ppl_compat::utc_now();
    store.save(op)
}
pub fn verify(prime: &mut Prime, store: &Store, id: &str) -> Result<Operation> {
    let _lock = store.lock()?;
    let mut op = store.load(id)?;
    if op.helper.is_some() {
        return Err(refused(
            "manual-helper recovery requires the existing CLI verification workflow",
        ));
    }
    reconcile(prime, store, &mut op)?;
    Ok(op)
}
