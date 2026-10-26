//! Read-only device reconciliation. Recovery never emits F7, EB, or EC.
use crate::{
    DeviceIdentity, ProgramImage, Result, Source, compare_source,
    delete_store::{Store, atomic_json},
    disk, refused,
};
use ppl_device::{ControlObject, DeleteTarget, DeviceError, Prime, TargetStatus, delete::Baseline};
pub fn upload(prime: &mut Prime, store: &Store, id: &str) -> Result<serde_json::Value> {
    ppl_device::validate_test_name(id)?;
    let _lock = store.lock()?;
    let path = store.root.join("uploads").join(format!("{id}.json"));
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).map_err(disk)?).map_err(disk)?;
    if record["schema_version"] != 1
        || record["operation_id"] != id
        || record["operation"] != "create_program"
    {
        return Err(refused("invalid upload recovery record"));
    }
    let device: DeviceIdentity = serde_json::from_value(record["device"].clone()).map_err(disk)?;
    device.require_reference()?;
    device.verify(prime)?;
    let name = record["name"]
        .as_str()
        .ok_or_else(|| refused("missing upload program name"))?
        .to_owned();
    let snapshot =
        std::fs::read(store.root.join("uploads").join(format!("{id}.utf16le"))).map_err(disk)?;
    if record["payload_sha256"] != ppl_compat::sha256_hex(&snapshot) {
        return Err(refused("upload source snapshot changed"));
    }
    let text = ppl_device::program::utf16(&snapshot)?;
    ProgramImage::new(&name, Source::text(&text, &name).map_err(refused)?)?.require_supported()?;
    let observation = match prime.read_program_exact(&name) {
        Ok(first) => {
            let second = prime.read_program_exact(&name)?;
            device.verify(prime)?;
            if first.data != second.data {
                return Err(refused("upload recovery reads changed"));
            }
            let comparison = compare_source(&first.data, &text)?;
            let state = if comparison.is_some() {
                "upload_verified"
            } else {
                "verification_failed"
            };
            serde_json::json!({"state":state,"comparison":comparison,"stored_sha256":ppl_compat::sha256_hex(&first.data)})
        }
        Err(DeviceError::Timeout(_)) => {
            let baseline = Baseline::from_info(&prime.info()?)?;
            let control =
                ControlObject::parse(ppl_device::DEFAULT_SETTINGS_ANCHOR).map_err(refused)?;
            let statuses = ppl_device::delete::verify_deleted_with(
                prime,
                &[DeleteTarget::program(&name)],
                &control,
                Some(&baseline),
            )?;
            device.verify(prime)?;
            serde_json::json!({"state":if statuses==[TargetStatus::ConfirmedDeleted]{"verified_absent"}else{"indeterminate"},"absence_probes":2})
        }
        Err(error) => return Err(error),
    };
    let history = store
        .root
        .join("uploads/history")
        .join(id)
        .join(format!("{}.json", ppl_device::fresh_test_name()));
    atomic_json(&history, &record).map_err(disk)?;
    record["state"] = observation["state"].clone();
    record["recovery"] = observation;
    record["reconciled_utc"] = serde_json::json!(ppl_compat::utc_now());
    atomic_json(&path, &record).map_err(disk)?;
    Ok(record)
}
