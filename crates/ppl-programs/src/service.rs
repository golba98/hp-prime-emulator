//! The exclusive owner's typed management dispatcher, shared by CLI and IDE.
use crate::{DeviceIdentity, ProgramImage, Result, Source, delete_store::Store, disk, refused};
use ppl_device::{Prime, control::ManagementAction as Action};
pub fn handle(
    prime: &mut Prime,
    expected_device: &str,
    action: &Action,
    store: &Store,
) -> Result<serde_json::Value> {
    // Validate source before any device traffic or deletion preparation.
    let image = match action {
        Action::Create { name, source, .. } | Action::PrepareReplacement { name, source } => {
            let image = ProgramImage::new(name, Source::text(source, name).map_err(refused)?)?;
            image.require_supported()?;
            Some(image)
        }
        _ => None,
    };
    if DeviceIdentity::read(prime)?.serial_id != expected_device {
        return Err(refused("management request targets another device"));
    }
    match action {
        Action::Create {
            confirmation_sha256,
            ..
        } => {
            let image = image.as_ref().unwrap();
            if *confirmation_sha256 != ppl_compat::sha256_hex(&image.payload()) {
                return Err(refused("upload authorization does not match exact source"));
            }
            serde_json::to_value(crate::create_new(prime, image, store)?).map_err(disk)
        }
        Action::PrepareReplacement { .. } => serde_json::to_value(crate::replacement::prepare(
            prime,
            image.as_ref().unwrap(),
            store,
        )?)
        .map_err(disk),
        Action::ApplyReplacement {
            operation_id,
            confirmation_new_sha256,
        } => {
            let plan: crate::replacement::Plan = serde_json::from_slice(
                &std::fs::read(store.directory(operation_id)?.join("replacement.json"))
                    .map_err(disk)?,
            )
            .map_err(disk)?;
            if *confirmation_new_sha256 != plan.new_sha256 {
                return Err(refused("replacement authorization source mismatch"));
            }
            crate::replacement::apply(prime, store, operation_id, true)
        }
        Action::VerifyDelete { operation_id } => {
            serde_json::to_value(crate::deletion::verify(prime, store, operation_id)?).map_err(disk)
        }
        Action::VerifyUpload { operation_id } => {
            crate::recovery::upload(prime, store, operation_id)
        }
        Action::VerifyReplacement { operation_id } => {
            crate::replacement::verify(prime, store, operation_id)
        }
        Action::PrepareDelete { name } => {
            serde_json::to_value(crate::deletion::prepare(prime, name, store)?).map_err(disk)
        }
        Action::ApplyDelete {
            operation_id,
            confirmation_old_sha256,
        } => serde_json::to_value(crate::deletion::apply(
            prime,
            store,
            operation_id,
            confirmation_old_sha256,
        )?)
        .map_err(disk),
    }
}
