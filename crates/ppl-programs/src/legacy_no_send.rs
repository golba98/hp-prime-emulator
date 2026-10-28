//! Narrow recovery of the reviewed version-1, single-target pre-EB failure.
//! Presence alone, a verification-only refusal, or an arbitrary error string is
//! insufficient. Missing/contradictory audit evidence keeps the operation blocked.
use crate::{
    Result,
    delete_store::{Operation, Store, atomic_json},
    disk,
};
use ppl_device::{Prime, delete::SubmissionState};
use serde_json::{Value, json};

const REVIEWED_REVISION: &str = "54b3b686b3e8520ffd5433d5312d032a2783d6b0";
const RULE: &str = "v1_single_user_target_pre_eb_quiet_refusal";

pub(crate) fn resolve(prime: &mut Prime, store: &Store, op: &Operation) -> Result<bool> {
    if op.schema_version != 1
        || op.class != "user"
        || op.mechanism != "usb_delete"
        || op.targets.len() != 1
        || op.targets[0].ty != "program"
        || op.helper.is_some()
        || op.helper_note.is_some()
        || op.helper_source.is_some()
        || op.helper_uploaded
        || op.completion.is_some()
        || !op.submission.is_empty()
        || op.authorization.as_deref() != Some("explicit_yes_flag")
        || op.usb_pid != 0x1541
        || op.firmware_build != Some(15157)
        || op.boot_version.as_deref() != Some("V0.050.640")
    {
        return Ok(false);
    }
    let dir = store.directory(&op.id)?;
    if dir.join("replacement.json").exists() {
        return Ok(false);
    }
    let original_path = dir.join("record-v1-original.json");
    let original = match std::fs::read(&original_path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::read(dir.join("operation.json")).map_err(disk)?
        }
        Err(e) => return Err(disk(e)),
    };
    let old: Value = serde_json::from_slice(&original).map_err(disk)?;
    if old["schema_version"] != 1
        || old["class"] != "user"
        || old["mechanism"] != "usb_delete"
        || old["authorization"] != "explicit_yes_flag"
        || !old["helper"].is_null()
        || !old["helper_source"].is_null()
        || old["targets"][0]["ty"] != op.targets[0].ty
        || old["targets"][0]["bytes"] != op.targets[0].bytes
        || old.get("submission").is_some()
        || old["id"] != op.id
        || old["state"] != "unresolved"
        || old["device_serial_id"] != op.device_serial_id
        || old["targets"][0]["name"] != op.targets[0].name
        || old["targets"][0]["sha256"] != op.targets[0].sha256
        || old["targets"].as_array().map(Vec::len) != Some(1)
    {
        return Ok(false);
    }
    let journal = std::fs::read(store.root.join("journal.jsonl")).map_err(disk)?;
    let mut entries = Vec::new();
    for line in journal
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
    {
        let entry: Value = serde_json::from_slice(line).map_err(disk)?;
        if entry["id"] == op.id {
            entries.push(entry);
        }
    }
    if entries.len() < 3 {
        return Ok(false);
    }
    let t = &op.targets[0];
    if entries.iter().any(|e| {
        e["device_serial_id"] != op.device_serial_id
            || e["mechanism"] != "usb_delete"
            || e["targets"].as_array().map(Vec::len) != Some(1)
            || e["targets"][0]["name"] != t.name
            || e["targets"][0]["ty"] != t.ty
            || e["targets"][0]["sha256"] != t.sha256
            || e["targets"][0]["bytes"] != t.bytes
            || e["submission"].as_array().is_some_and(|s| !s.is_empty())
    }) {
        return Ok(false);
    }
    // The initial sequence, not a later recovery error, establishes where the
    // original single attempt failed. A modern or repeated intent is excluded.
    if entries[0]["phase"] != "prepared"
        || entries[1]["phase"] != "submitting"
        || entries[0].get("submission").is_some()
        || entries[1].get("submission").is_some()
        || !entries[0]["error"].is_null()
        || !entries[1]["error"].is_null()
        || entries[2..].iter().any(|e| e["phase"] != "unresolved")
    {
        return Ok(false);
    }
    let refusal = format!(
        "refused: `{}` exists on the calculator (its reply arrived late); it will not be overwritten",
        t.name
    );
    let combined = format!("{refusal}; verification: {refusal}");
    if entries[2]["error"] != combined || entries[2].get("submission").is_some() {
        return Ok(false);
    }
    let backups = store.backups(op)?;
    // Independent current evidence corroborates exact unchanged target identity;
    // the reviewed error path, NOT presence, proves that no EB was attempted.
    for _ in 0..2 {
        let current = prime.read_program_exact(&t.name)?;
        if current.data != backups[0].data {
            return Ok(false);
        }
    }
    crate::DeviceIdentity {
        serial_id: op.device_serial_id.clone(),
        usb_pid: op.usb_pid,
        firmware_build: op.firmware_build,
        boot_version: op.boot_version.clone(),
    }
    .verify(prime)?;
    atomic_json(&dir.join("legacy-no-send-evidence.json"),&json!({
        "schema_version":1,"rule":RULE,"operation_id":op.id,"device_serial_id":op.device_serial_id,"target":t,
        "reviewed_revision":REVIEWED_REVISION,
        "reviewed_git_blobs":{"device_delete":"60d0029ef7489cf02d497a0d634e2e96e0fbd72a","device_lib":"6e29744c15b3fa0e0be547bb88b5443b379747b7","cli_user_flow":"1a6e6ae7f52f1a0720507cd162986341cacdb3ce"},
        "original_record_sha256":ppl_compat::sha256_hex(&original),"audit_snapshot_sha256":ppl_compat::sha256_hex(&journal),
        "original_attempt":&entries[..3],"basis":"reviewed single-target legacy pre-EB control flow plus original audit, verified backup and two unchanged current reads; not a USB capture",
        "derived_submission":SubmissionState::NotSent,"hardware_mutations":0,"reconciled_utc":ppl_compat::utc_now()
    })).map_err(disk)?;
    Ok(true)
}
