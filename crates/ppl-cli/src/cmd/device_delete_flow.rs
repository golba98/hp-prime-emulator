//! Helper execution, read-only reconciliation and finite artifact cleanup.
use super::*;

pub(super) fn manual(
    p: &mut Prime,
    d: &ppl_device::Deletion,
    store: &Store,
    op: &mut Operation,
    opts: &DeleteOptions,
    prepare_only: bool,
    json_output: bool,
) -> Result<(), DeviceError> {
    let control = op.control()?;
    op.helper = Some(d.helper.clone());
    op.helper_note = Some(d.helper_note.clone());
    op.helper_source = Some(d.source.clone());
    op.cleanup_state = "pending".into();
    op.state = "submitting".into();
    store.save(op)?;
    let upload = ppl_device::upload_deletion_helper(p, d, &control)?;
    if upload.readback != ppl_device::ReadBack::Matches {
        return Err(refused(
            "helper upload read-back did not match; do not retry blindly",
        ));
    }
    op.helper_uploaded = true;
    op.state = "awaiting_run".into();
    store.save(op)?;
    if prepare_only {
        return Ok(());
    }
    if !json_output {
        eprintln!(
            "Run `{}` ONCE on the calculator; waiting for its result.",
            d.helper
        );
    }
    let note = ppl_device::await_deletion_note(p, d, opts, |_| {})?;
    op.completion = note.map(|w| w.text);
    // Persist completion as soon as observed, before independent absence checks.
    store.save(op)?;
    let statuses = ppl_device::verify_deleted(p, &op.targets()?, &control)?;
    apply_statuses(op, &statuses);
    op.state = if op.confirmed() && op.completion.as_deref() == Some("done") {
        "verified"
    } else {
        "unresolved"
    }
    .into();
    if op.completion.as_deref() != Some("done") {
        op.error =
            Some("helper did not produce a correlated terminal result before the deadline".into());
    }
    store.save(op)?;
    if op.confirmed() && op.completion.as_deref() == Some("done") {
        archive_artifacts(p, store, op)?;
        retire_confirmed(store, op)?;
        let identity_log_start = p.log.len();
        let info = p.info();
        p.log.truncate(identity_log_start);
        let info = info?;
        if direct_profile(p, &info) {
            cleanup(p, store, op)?;
        }
    }
    Ok(())
}
pub(super) fn require_device(op: &Operation, serial: &str) -> Result<(), DeviceError> {
    if op.device_serial_id != serial {
        Err(refused("operation belongs to another physical calculator"))
    } else {
        Ok(())
    }
}
pub(super) fn retire_confirmed(store: &Store, op: &Operation) -> Result<(), DeviceError> {
    for (target, b) in op.targets.iter().zip(store.backups(op)?) {
        if target.status == "confirmed_deleted" {
            records::retire(&op.runs_dir, &op.device_serial_id, &b)?;
        }
    }
    Ok(())
}
pub(super) fn reconcile(
    p: &mut Prime,
    store: &Store,
    op: &mut Operation,
) -> Result<(), DeviceError> {
    // Common read-only recovery preserves identity, backups and submission evidence.
    ppl_programs::deletion::reconcile(p, store, op)?;
    if op.confirmed() {
        if op.helper.is_some() && op.cleanup_state != "complete" {
            archive_artifacts(p, store, op)?;
        }
        retire_confirmed(store, op)?;
    }
    Ok(())
}
fn archive_artifacts(p: &mut Prime, store: &Store, op: &mut Operation) -> Result<(), DeviceError> {
    let Some(note) = op.helper_note.clone() else {
        return Ok(());
    };
    let file = match p.request_file(FileType::Note, &note)? {
        Probe::Present(f) if f.name == note && f.ty == FileType::Note && f.crc_ok => f,
        _ => return Err(refused("helper evidence unavailable; artifacts retained")),
    };
    let expected = if op
        .helper_source
        .as_deref()
        .is_some_and(|s| s.contains("@@PPLDELETE|"))
    {
        format!("@@PPLDELETE|{}|done", op.id)
    } else {
        "done".into()
    };
    if ppl_device::normalize_program_text(&file.text()) != expected {
        return Err(refused("helper is not terminal; artifacts retained"));
    }
    op.completion = Some("done".into());
    if !op.artifacts.iter().any(|a| a.name == note) {
        op.artifacts.push(records::Artifact {
            name: note,
            ty: "note".into(),
            sha256: ppl_compat::sha256_hex(&file.data),
            text: file.text(),
            status: "present".into(),
        });
    }
    if let (Some(helper), Some(source)) = (&op.helper, &op.helper_source) {
        match p.request_file(FileType::Program, helper)? {
            Probe::Present(f)
                if f.name == *helper
                    && f.ty == FileType::Program
                    && f.crc_ok
                    && ppl_device::normalize_program_text(&f.text())
                        == ppl_device::normalize_program_text(source) =>
            {
                if !op.artifacts.iter().any(|a| a.name == *helper) {
                    op.artifacts.push(records::Artifact {
                        name: helper.clone(),
                        ty: "program".into(),
                        sha256: ppl_compat::sha256_hex(&f.data),
                        text: f.text(),
                        status: "present".into(),
                    });
                }
            }
            Probe::NoReply { .. } => {
                if ppl_device::verify_deleted(p, &[DeleteTarget::program(helper)], &op.control()?)?
                    != [TargetStatus::ConfirmedDeleted]
                {
                    return Err(refused(
                        "helper absence could not be confirmed; retaining evidence",
                    ));
                }
            }
            _ => return Err(refused("helper changed; refusing artifact cleanup")),
        }
    }
    // Full note contents and hashes are synced before they may be deleted.
    store.save(op)
}
pub(super) fn cleanup(p: &mut Prime, store: &Store, op: &mut Operation) -> Result<(), DeviceError> {
    if !op.confirmed() {
        return Err(refused("unresolved deletion evidence must be retained"));
    }
    if op.helper.is_none() {
        op.cleanup_state = "not_required".into();
        return store.save(op);
    }
    if op.cleanup_state == "complete" {
        return Ok(());
    }
    if op.artifacts.is_empty() {
        archive_artifacts(p, store, op)?;
    }
    let targets: Vec<_> = op
        .artifacts
        .iter()
        .map(|a| DeleteTarget {
            name: a.name.clone(),
            ty: FileType::parse(&a.ty).unwrap_or(FileType::Other(255)),
        })
        .collect();
    let statuses = ppl_device::verify_deleted(p, &targets, &op.control()?)?;
    let mut present = Vec::new();
    for (a, status) in op.artifacts.iter_mut().zip(statuses) {
        a.status = status_str(&status).into();
        if status == TargetStatus::StillPresent {
            present.push(DeleteTarget {
                name: a.name.clone(),
                ty: FileType::parse(&a.ty).unwrap(),
            });
        }
    }
    if !present.is_empty() {
        if matches!(op.cleanup_state.as_str(), "submitting" | "unresolved") {
            op.cleanup_state = "unresolved".into();
            return Err(refused(
                "cleanup submission is unresolved; no destructive command was retried",
            ));
        }
        let deletion = ppl_device::prepare_deletion(p, &present, &op.control()?)?;
        for b in &deletion.backups {
            let artifact = op
                .artifacts
                .iter()
                .find(|a| a.name == b.name && a.ty == b.ty.label())
                .ok_or_else(|| refused("unrecorded cleanup target"))?;
            if artifact.sha256 != ppl_compat::sha256_hex(&b.data) {
                return Err(refused("artifact content changed since archival"));
            }
        }
        // Persist raw cleanup backups separately; this operation creates no helper/note.
        let archive = store.directory(&op.id)?.join("cleanup-backups");
        std::fs::create_dir_all(&archive).map_err(disk)?;
        for b in &deletion.backups {
            let path = archive.join(format!("{}-{}.bin", b.ty.label(), b.name));
            if path.exists() {
                if std::fs::read(&path).map_err(disk)? != b.data {
                    return Err(refused("cleanup backup differs"));
                }
            } else {
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(disk)?;
                f.write_all(&b.data)
                    .and_then(|_| f.sync_all())
                    .map_err(disk)?;
            }
        }
        records::sync_dir(&archive).map_err(disk)?;
        op.cleanup_state = "submitting".into();
        store.save(op)?;
        let guard = ppl_device::authorize_usb_deletion(p, &deletion, &op.control()?)?;
        let report = ppl_device::delete_via_usb(p, guard, |_| {})?;
        for r in report.targets {
            if let Some(a) = op
                .artifacts
                .iter_mut()
                .find(|a| a.name == r.name && a.ty == r.ty.label())
            {
                a.status = status_str(&r.status).into();
            }
        }
        if let Some(error) = report.error {
            op.error = Some(error);
        }
    }
    let all = op.artifacts.iter().all(|a| a.status == "confirmed_deleted");
    op.cleanup_state = if all { "complete" } else { "unresolved" }.into();
    if all {
        op.state = "completed".into();
        op.error = None;
    }
    store.save(op)
}

#[cfg(test)]
#[path = "device_delete_flow_tests.rs"]
mod tests;
