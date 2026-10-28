//! Explicit non-atomic replacement. Preparing never changes calculator data.
//! Applying authorizes one delete and one create, never an automatic rollback.
use crate::delete_store::{Operation, Store, Target, atomic_json};
use crate::{DeviceIdentity, ProgramImage, Result, disk, refused, upload_new};
use ppl_device::{
    ControlObject, DeleteTarget, NamePolicy, Prime, TargetStatus,
    delete::{Baseline, SubmissionState},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub schema_version: u32,
    pub operation_id: String,
    pub device: DeviceIdentity,
    pub name: String,
    pub old_sha256: String,
    pub new_sha256: String,
    pub sizes: crate::EncodedSize,
    pub atomic: bool,
    pub checks: ppl_engine::checking::CheckReport,
}
fn anchor() -> Result<ControlObject> {
    ControlObject::parse(ppl_device::DEFAULT_SETTINGS_ANCHOR).map_err(refused)
}

pub fn prepare(prime: &mut Prime, image: &ProgramImage, store: &Store) -> Result<Plan> {
    image.require_supported()?; // An unsupported new payload can NEVER cause a deletion.
    let checks = ppl_engine::checking::check(
        image.source.clone(),
        ppl_engine::checking::configured_firmware().as_ref(),
        &Default::default(),
    );
    if !checks.accepted() {
        return Err(refused(
            "replacement source checking failed; inspect local and configured firmware diagnostics",
        ));
    }
    let _lock = store.lock()?;
    let device = DeviceIdentity::read(prime)?;
    device.require_reference()?;
    let targets = [DeleteTarget::program(&image.name)];
    store.reject_pending(&device.serial_id, &targets)?;
    let info = prime.info()?;
    let baseline = Baseline::from_info(&info)?;
    let deletion = ppl_device::prepare_deletion_with(
        prime,
        &targets,
        &anchor()?,
        NamePolicy::User,
        Some(&baseline),
    )?;
    let backup = &deletion.backups[0];
    let now = ppl_compat::utc_now();
    let op = Operation {
        schema_version: 1,
        id: deletion.helper.clone(),
        device_serial_id: device.serial_id.clone(),
        usb_pid: device.usb_pid,
        firmware_build: device.firmware_build,
        boot_version: device.boot_version.clone(),
        control: anchor()?.to_string(),
        state: "prepared".into(),
        mechanism: "usb_delete".into(),
        created_utc: now.clone(),
        updated_utc: now,
        targets: vec![Target {
            name: image.name.clone(),
            ty: "program".into(),
            sha256: ppl_compat::sha256_hex(&backup.data),
            bytes: backup.data.len(),
            status: "not_submitted".into(),
        }],
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
    store.create(&op, &deletion.backups)?;
    store.backups(&op)?;
    let plan = Plan {
        schema_version: 1,
        operation_id: op.id.clone(),
        device,
        name: image.name.clone(),
        old_sha256: op.targets[0].sha256.clone(),
        new_sha256: ppl_compat::sha256_hex(&image.payload()),
        sizes: image.sizes.clone(),
        atomic: false,
        checks,
    };
    let dir = store.directory(&op.id)?;
    // Create-new, sync, and verify the source snapshot before offering apply.
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join("replacement-source.utf16le"))
        .map_err(disk)?;
    file.write_all(&image.payload())
        .and_then(|_| file.sync_all())
        .map_err(disk)?;
    atomic_json(&dir.join("replacement.json"), &plan).map_err(disk)?;
    Ok(plan)
}

/// Explicit authorization is supplied by the frontend for exactly this plan.
/// A prepared plan is the only retryable state; submission intent consumes it.
pub fn apply(
    prime: &mut Prime,
    store: &Store,
    id: &str,
    authorized: bool,
) -> Result<serde_json::Value> {
    if !authorized {
        return Err(refused(
            "replacement requires explicit authorization of this operation ID",
        ));
    }
    let _lock = store.lock()?;
    let mut op = store.load(id)?;
    if op.state != "prepared" || op.authorization.is_some() {
        return Err(refused(
            "replacement already attempted or unresolved; reconcile read-only, never retry apply",
        ));
    }
    store.reject_other_pending(&op.device_serial_id, &op.targets()?, Some(id))?;
    if op.targets.len() != 1 || op.targets[0].ty != "program" {
        return Err(refused("management apply requires one program target"));
    }
    let dir = store.directory(id)?;
    let plan: Plan =
        serde_json::from_slice(&std::fs::read(dir.join("replacement.json")).map_err(disk)?)
            .map_err(disk)?;
    let bytes = std::fs::read(dir.join("replacement-source.utf16le")).map_err(disk)?;
    if plan.schema_version != 1
        || plan.operation_id != id
        || plan.device.serial_id != op.device_serial_id
        || plan.name != op.targets[0].name
        || plan.old_sha256 != op.targets[0].sha256
        || plan.device.usb_pid != op.usb_pid
        || plan.device.firmware_build != op.firmware_build
        || plan.device.boot_version != op.boot_version
        || ppl_compat::sha256_hex(&bytes) != plan.new_sha256
    {
        return Err(refused(
            "replacement plan identity or immutable snapshot mismatch",
        ));
    }
    let text = ppl_device::program::utf16(&bytes)?;
    let image = ProgramImage::new(
        &plan.name,
        ppl_engine::source::Source::text(&text, &plan.name).map_err(refused)?,
    )?;
    image.require_supported()?;
    if plan.checks.source_sha256 != image.source.identity.source_sha256 {
        return Err(refused(
            "replacement compiler report source identity mismatch",
        ));
    }
    if !plan.checks.accepted() {
        return Err(refused(
            "replacement source was not accepted by the configured checks",
        ));
    }
    plan.device.require_reference()?;
    plan.device.verify(prime)?;
    let backups = store.backups(&op)?;
    let baseline = Baseline::from_info(&prime.info()?)?;
    let deletion = ppl_device::prepare_deletion_with(
        prime,
        &[DeleteTarget::program(&plan.name)],
        &op.control()?,
        NamePolicy::User,
        Some(&baseline),
    )?;
    if deletion.backups[0].data != backups[0].data {
        return Err(refused("destination changed since replacement preparation"));
    }
    atomic_json(&dir.join("authorization.json"),&serde_json::json!({
        "device":plan.device,"type":"program","name":plan.name,"old_sha256":plan.old_sha256,"new_sha256":plan.new_sha256,
        "scope":"one non-atomic delete-then-create attempt; no retry or automatic restoration"
    })).map_err(disk)?;
    op.authorization = Some("explicit_replacement_plan".into());
    op.state = "submitting".into();
    op.updated_utc = ppl_compat::utc_now();
    store.save(&op)?;
    let result = crate::submit_deletion(prime, &deletion, store, &mut op);
    let report = match result {
        Ok(r) => r,
        Err(error) => {
            op.state = "not_submitted".into();
            op.error = Some(error.to_string());
            store.save(&op)?;
            return Err(error);
        }
    };
    let never_sent = report
        .submission
        .iter()
        .all(|s| *s == SubmissionState::NotSent);
    op.submission = report.submission;
    let deleted = report
        .targets
        .iter()
        .all(|t| t.status == TargetStatus::ConfirmedDeleted);
    op.error = report.error;
    op.targets[0].status = if deleted {
        "confirmed_deleted"
    } else {
        "unknown"
    }
    .into();
    if !deleted || never_sent || op.error.is_some() {
        op.state = if never_sent {
            "not_submitted"
        } else {
            "unresolved"
        }
        .into();
        store.save(&op)?;
        return Err(refused(format!(
            "replacement stopped before create: {}",
            op.error.as_deref().unwrap_or("deletion not verified")
        )));
    }
    // Vulnerable intermediate state: old object is absent and the raw backup
    // exists. A crash here MUST NOT silently complete the replacement on resume.
    op.state = "replacement_upload_submitting".into();
    store.save(&op)?;
    let uploaded = upload_new(prime, &plan.device, &image);
    let output = match uploaded {
        Ok(r) => {
            op.state = if r.state == "upload_verified" {
                "completed"
            } else {
                "unresolved"
            }
            .into();
            op.error = r.error.clone();
            serde_json::to_value(r).map_err(disk)?
        }
        Err(e) => {
            op.state = "unresolved".into();
            op.error = Some(e.to_string());
            serde_json::json!({"state":"indeterminate","error":e.to_string()})
        }
    };
    op.updated_utc = ppl_compat::utc_now();
    store.save(&op)?;
    Ok(
        serde_json::json!({"operation_id":id,"atomic":false,"phase":op.state,"upload":output,"backup_preserved":true}),
    )
}

/// Replacement recovery distinguishes the new destination from the deleted
/// original; generic absence verification must never complete this transaction.
pub fn verify(prime: &mut Prime, store: &Store, id: &str) -> Result<serde_json::Value> {
    let _lock = store.lock()?;
    let mut op = store.load(id)?;
    let dir = store.directory(id)?;
    let plan: Plan =
        serde_json::from_slice(&std::fs::read(dir.join("replacement.json")).map_err(disk)?)
            .map_err(disk)?;
    if plan.operation_id != id
        || plan.name != op.targets[0].name
        || plan.device.serial_id != op.device_serial_id
    {
        return Err(refused("replacement recovery identity mismatch"));
    }
    plan.device.verify(prime)?;
    let source = prime.read_program_exact(&plan.name)?;
    let hash = ppl_compat::sha256_hex(&source.data);
    let snapshot = std::fs::read(dir.join("replacement-source.utf16le")).map_err(disk)?;
    if ppl_compat::sha256_hex(&snapshot) != plan.new_sha256 {
        return Err(refused("replacement source snapshot changed"));
    }
    let expected = ppl_device::program::utf16(&snapshot)?;
    let second = prime.read_program_exact(&plan.name)?;
    plan.device.verify(prime)?;
    if second.data != source.data {
        return Err(refused("replacement recovery reads changed"));
    }
    let comparison = crate::compare_source(&source.data, &expected)?;
    if comparison.is_some()
        && op.authorization.as_deref() == Some("explicit_replacement_plan")
        && op.targets.iter().all(|t| t.status == "confirmed_deleted")
        && op.submission.iter().any(|s| *s != SubmissionState::NotSent)
    {
        op.state = "completed".into();
        op.error = None;
        op.updated_utc = ppl_compat::utc_now();
        store.save(&op)?;
    }
    Ok(
        serde_json::json!({"operation_id":id,"observed_sha256":hash,"original_present":hash==plan.old_sha256,
        "new_source_present":comparison.is_some(),"comparison":comparison,"phase":op.state,
        "history_preserved":true,"automatic_retry":false,"note":"read-only observation; no submission is inferred or retried"}),
    )
}
