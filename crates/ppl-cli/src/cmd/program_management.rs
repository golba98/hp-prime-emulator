//! Shared management CLI. Preparation and observation never mutate the device.
use super::device::DeviceOpts;
use clap::Subcommand;
use ppl_programs::{ProgramImage, delete_store::Store};
use std::path::PathBuf;
#[derive(Debug, Subcommand)]
pub enum Action {
    /// Inspect exact encoding and size without opening USB.
    Inspect { name: String, file: PathBuf },
    /// Prepare a backed-up, checked, non-atomic replacement; no device writes.
    Prepare { name: String, file: PathBuf },
    /// Execute one separately authorized plan; never retries or auto-restores.
    Apply {
        #[arg(long)]
        operation_id: String,
        #[arg(long)]
        yes: bool,
    },
    /// Reconcile an upload with strict readback or independently verified absence.
    VerifyUpload {
        #[arg(long)]
        operation_id: String,
    },
    /// Observe replacement readback; never resubmits or force-clears history.
    Verify {
        #[arg(long)]
        operation_id: String,
    },
}
pub fn run(options: &DeviceOpts, action: &Action) -> u8 {
    let result = (|| -> ppl_programs::Result<serde_json::Value> {
        if let Action::Inspect { name, file } = action {
            let image = ProgramImage::file(name, file)?;
            return Ok(
                serde_json::json!({"operation":"inspect_program","name":name,"encoding":image.source.identity.encoding,
                "sizes":image.sizes,"input_sha256":image.source.identity.input_sha256,"source_unchanged":true,"uploaded":false}),
            );
        }
        // Validate input before even opening USB.
        let image = if let Action::Prepare { name, file } = action {
            let image = ProgramImage::file(name, file)?;
            image.require_supported()?;
            Some(image)
        } else {
            None
        };
        if matches!(action, Action::Apply { yes: false, .. }) {
            return Err(ppl_programs::refused(
                "apply requires --yes for exactly this replacement operation",
            ));
        }
        if options.control_socket.is_some() {
            use ppl_device::control::{ManagementAction as M, Request};
            let selected = options.device.as_ref().ok_or_else(|| {
                ppl_programs::refused("--control-socket management requires --device serial-id")
            })?;
            let request = match action {
                Action::Prepare { .. } => {
                    let image = image.as_ref().unwrap();
                    M::PrepareReplacement {
                        name: image.name.clone(),
                        source: image.source.original_text.clone(),
                    }
                }
                Action::Apply { operation_id, .. } => {
                    let store = Store::new(std::path::Path::new(
                        super::device_delete::DEFAULT_DELETE_DIR,
                    ));
                    let plan: ppl_programs::replacement::Plan = serde_json::from_slice(
                        &std::fs::read(store.directory(operation_id)?.join("replacement.json"))
                            .map_err(ppl_programs::disk)?,
                    )
                    .map_err(ppl_programs::disk)?;
                    M::ApplyReplacement {
                        operation_id: operation_id.clone(),
                        confirmation_new_sha256: plan.new_sha256,
                    }
                }
                Action::VerifyUpload { operation_id } => M::VerifyUpload {
                    operation_id: operation_id.clone(),
                },
                Action::Verify { operation_id } => M::VerifyReplacement {
                    operation_id: operation_id.clone(),
                },
                Action::Inspect { .. } => unreachable!(),
            };
            let response = super::device_control::request(
                options,
                &Request::Manage {
                    device_serial_id: selected.clone(),
                    action: request,
                },
            );
            if !response.report["error"].is_null() {
                return Err(ppl_programs::refused(response.report["error"].to_string()));
            }
            return Ok(response.report["result"].clone());
        }
        let mut prime = ppl_programs::open_selected(options.device.as_deref())?;
        let store = Store::new(std::path::Path::new(
            super::device_delete::DEFAULT_DELETE_DIR,
        ));
        match action {
            Action::Prepare { .. } => serde_json::to_value(ppl_programs::replacement::prepare(
                &mut prime,
                image.as_ref().unwrap(),
                &store,
            )?)
            .map_err(ppl_programs::disk),
            Action::Apply { operation_id, yes } => {
                ppl_programs::replacement::apply(&mut prime, &store, operation_id, *yes)
            }
            Action::VerifyUpload { operation_id } => {
                ppl_programs::recovery::upload(&mut prime, &store, operation_id)
            }
            Action::Verify { operation_id } => {
                ppl_programs::replacement::verify(&mut prime, &store, operation_id)
            }
            Action::Inspect { .. } => unreachable!(),
        }
    })();
    match result {
        Ok(value) => {
            println!(
                "{}",
                if options.json {
                    value.to_string()
                } else {
                    serde_json::to_string_pretty(&value).unwrap()
                }
            );
            let verified = match action {
                Action::Inspect { .. } | Action::Prepare { .. } => true,
                Action::Apply { .. } => {
                    value["phase"] == "completed" && value["upload"]["state"] == "upload_verified"
                }
                Action::Verify { .. } => {
                    value["phase"] == "completed" && value["new_source_present"] == true
                }
                Action::VerifyUpload { .. } => value["state"] == "upload_verified",
            };
            if verified { 0 } else { 5 }
        }
        Err(error) => {
            if options.json {
                println!(
                    "{}",
                    serde_json::json!({"status":"refused","error":error.to_string(),"uploaded":false,"execution_confirmed":false})
                );
            } else {
                eprintln!("error: {error}");
            }
            5
        }
    }
}
