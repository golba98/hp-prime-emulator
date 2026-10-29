//! One-click "Send to calculator": the dialogs and their state.
//!
//! The flow only decides what to ask and what to start; the app owns the
//! editor and the device link and acts on the returned [`SendEvent`]s.
//! Deleting an existing program is never part of a plain send: a replacement
//! is prepared (and backed up) by the worker, then needs its own click on
//! "Replace", bound to the exact source hash shown in the editor.

use super::device::{JobError, JobOutput};
use super::editor::TabId;
use crate::theme::{ACCENT_AMBER, ACCENT_GREEN, ACCENT_RED, TEXT_MUTED};
use egui::RichText;
use ppl_programs::replacement::Plan;
use std::path::PathBuf;

/// Exactly what will be sent: frozen when the user clicked Send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendRequest {
    pub tab: TabId,
    pub path: PathBuf,
    pub name: String,
    pub source: String,
    /// SHA-256 of the UTF-16LE payload, which authorizes the write.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    Sent {
        name: String,
        replaced: bool,
    },
    Failed {
        name: String,
        message: String,
        hint: Option<String>,
        /// Journal operation to verify from the Advanced window.
        recovery_id: Option<String>,
    },
}

#[derive(Debug, Default)]
pub enum SendStage {
    #[default]
    Idle,
    /// Waiting for the background check of this tab to finish.
    WaitingForCheck(TabId),
    /// First send of a file: confirm or edit the calculator program name.
    AskName {
        tab: TabId,
        path: PathBuf,
        name: String,
        error: Option<String>,
    },
    /// The emulator could not test some features.
    ConfirmUntested {
        request: SendRequest,
        untested: Vec<String>,
    },
    Sending(SendRequest),
    ConfirmReplace {
        request: SendRequest,
        plan: Box<Plan>,
    },
    Replacing(SendRequest),
    Finished(SendOutcome),
}

/// What the user decided in a dialog; the app carries it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendEvent {
    /// Remember `name` for `path` and continue the send.
    NameChosen {
        tab: TabId,
        path: PathBuf,
        name: String,
    },
    /// Start the device job for this request.
    Start(SendRequest),
    /// Apply the prepared replacement (the app re-checks the editor first).
    Replace {
        request: SendRequest,
        operation_id: String,
        new_sha256: String,
    },
    OpenAdvanced(String),
    CompareScreens,
    /// Retire the durable, unsubmitted plan as well as its dialog.
    CancelReplacement(String),
}

#[derive(Debug, Default)]
pub struct SendFlow {
    pub stage: SendStage,
}

/// Payload hash used by the guarded service to authorize a write.
pub fn payload_sha256(source: &str) -> String {
    let bytes: Vec<u8> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
    ppl_engine::source::hash(&bytes)
}

/// Calculator program name suggested for a file: its stem, reduced to
/// `[A-Za-z0-9_]`, starting with a letter, at most 16 characters.
pub fn suggested_name(path: &std::path::Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let mut name: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        name.insert(0, 'P');
    }
    name.truncate(ppl_device::MAX_NAME_LEN.min(16));
    name
}

impl SendFlow {
    pub fn active(&self) -> bool {
        !matches!(self.stage, SendStage::Idle | SendStage::Finished(_))
    }

    pub fn waiting_for(&self) -> Option<TabId> {
        match self.stage {
            SendStage::WaitingForCheck(tab) => Some(tab),
            _ => None,
        }
    }

    /// Routes a finished send or replace job into the next stage.
    pub fn on_result(&mut self, result: Result<JobOutput, JobError>) {
        let stage = std::mem::take(&mut self.stage);
        let (request, replacing) = match stage {
            SendStage::Sending(r) => (r, false),
            SendStage::Replacing(r) => (r, true),
            other => {
                // Not expecting a result (e.g. cancelled); keep what we had.
                self.stage = other;
                return;
            }
        };
        let name = request.name.clone();
        let failed = |message: String, hint: Option<String>, recovery_id: Option<String>| {
            SendStage::Finished(SendOutcome::Failed {
                name: name.clone(),
                message,
                hint,
                recovery_id,
            })
        };
        self.stage = match result {
            Err(e) => failed(e.message, e.hint.map(str::to_string), None),
            Ok(JobOutput::ReplacementPrepared(plan)) if !replacing => {
                SendStage::ConfirmReplace { request, plan }
            }
            Ok(JobOutput::Created(r)) if r.state == "upload_verified" => {
                SendStage::Finished(SendOutcome::Sent {
                    name: r.name,
                    replaced: false,
                })
            }
            Ok(JobOutput::Created(r)) => failed(
                format!(
                    "The upload could not be verified ({}): {}",
                    r.state.replace('_', " "),
                    r.error.as_deref().unwrap_or("no details")
                ),
                Some("Open Advanced to check what reached the calculator.".into()),
                r.operation_id,
            ),
            Ok(JobOutput::Replaced(v)) if v["phase"] == "completed" => {
                SendStage::Finished(SendOutcome::Sent {
                    name: request.name,
                    replaced: true,
                })
            }
            Ok(JobOutput::Replaced(v)) => failed(
                format!(
                    "The replacement did not finish ({}). The old version is backed up.",
                    v["phase"].as_str().unwrap_or("unknown").replace('_', " ")
                ),
                Some("Open Advanced and use Verify replacement to see the current state.".into()),
                v["operation_id"].as_str().map(str::to_string),
            ),
            Ok(other) => failed(format!("Unexpected device reply: {other:?}"), None, None),
        };
    }

    /// Cancels a pending replacement when the editor no longer holds the
    /// source it was prepared for.
    pub fn invalidate_if_changed(
        &mut self,
        tab: TabId,
        current_source: Option<&str>,
    ) -> Option<String> {
        if let SendStage::ConfirmReplace { request, plan } = &self.stage {
            if request.tab == tab && current_source != Some(request.source.as_str()) {
                let operation_id = plan.operation_id.clone();
                self.stage = SendStage::Finished(SendOutcome::Failed {
                    name: request.name.clone(),
                    message: "The program changed after the replacement was prepared, \
                              so nothing was replaced."
                        .into(),
                    hint: Some("Send again to replace it with the current version.".into()),
                    recovery_id: None,
                });
                return Some(operation_id);
            }
        }
        None
    }

    /// Draws the current dialog and returns the user's decision.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<SendEvent> {
        let mut event = None;
        let mut close = false;
        let title = "Send to calculator";
        let window = || {
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .default_width(420.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        };
        match &mut self.stage {
            SendStage::Idle => {}
            SendStage::WaitingForCheck(_) => {
                window().show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Checking the program before sending…");
                    });
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            }
            SendStage::AskName {
                tab,
                path,
                name,
                error,
            } => {
                window().show(ctx, |ui| {
                    ui.label("Name of the program on the calculator:");
                    let edit = ui.text_edit_singleline(name);
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.label(
                        RichText::new(
                            "1–16 letters, digits or _, starting with a letter. \
                             This file will be sent under this name from now on.",
                        )
                        .size(11.0)
                        .color(TEXT_MUTED),
                    );
                    if let Some(e) = error {
                        ui.colored_label(ACCENT_RED, e.as_str());
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Send").clicked() || enter {
                            match ppl_device::validate_program_name(name.trim()) {
                                Ok(()) => {
                                    event = Some(SendEvent::NameChosen {
                                        tab: *tab,
                                        path: path.clone(),
                                        name: name.trim().to_string(),
                                    })
                                }
                                Err(e) => *error = Some(e.to_string()),
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            SendStage::ConfirmUntested { request, untested } => {
                window().show(ctx, |ui| {
                    ui.colored_label(
                        ACCENT_AMBER,
                        format!(
                            "{} feature(s) could not be tested in the emulator.",
                            untested.len()
                        ),
                    );
                    ui.label("They may still work on the calculator:");
                    for line in untested.iter().take(8) {
                        ui.label(RichText::new(format!("• {line}")).size(11.5));
                    }
                    if untested.len() > 8 {
                        ui.label(format!("… and {} more", untested.len() - 8));
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Send anyway").clicked() {
                            event = Some(SendEvent::Start(request.clone()));
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            SendStage::Sending(request) | SendStage::Replacing(request) => {
                window().show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("Sending {} to the calculator…", request.name));
                    });
                    ui.label(
                        RichText::new(
                            "Keep the calculator connected. This can take up to a minute.",
                        )
                        .size(11.0)
                        .color(TEXT_MUTED),
                    );
                });
            }
            SendStage::ConfirmReplace { request, plan } => {
                window().show(ctx, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} already exists on the calculator.",
                            request.name
                        ))
                        .strong(),
                    );
                    ui.label("Replace it with the version in the editor?");
                    let backup = std::path::Path::new(ppl_programs::DEFAULT_STORE_DIR)
                        .join("operations")
                        .join(&plan.operation_id);
                    ui.label(
                        RichText::new(format!(
                            "The calculator's current copy has been backed up to {}.",
                            backup.display()
                        ))
                        .size(11.0)
                        .color(TEXT_MUTED),
                    );
                    if !plan.checks.accepted() {
                        ui.colored_label(ACCENT_RED, "The source checks did not pass.");
                    }
                    ui.horizontal(|ui| {
                        let replace =
                            egui::Button::new(RichText::new("Replace").color(ACCENT_AMBER));
                        if ui.add(replace).clicked() {
                            event = Some(SendEvent::Replace {
                                request: request.clone(),
                                operation_id: plan.operation_id.clone(),
                                new_sha256: plan.new_sha256.clone(),
                            });
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            SendStage::Finished(outcome) => {
                window().show(ctx, |ui| match outcome {
                    SendOutcome::Sent { name, replaced } => {
                        ui.colored_label(
                            ACCENT_GREEN,
                            if *replaced {
                                format!("✓ {name} replaced on the calculator")
                            } else {
                                format!("✓ {name} sent to the calculator")
                            },
                        );
                        ui.label(
                            RichText::new("Verified by reading it back from the calculator.")
                                .size(11.0)
                                .color(TEXT_MUTED),
                        );
                        ui.horizontal(|ui| {
                            if ui
                                .button("Compare screens")
                                .on_hover_text(
                                    "Run the program on the calculator first, then compare \
                                         its screen with the emulator",
                                )
                                .clicked()
                            {
                                event = Some(SendEvent::CompareScreens);
                                close = true;
                            }
                            if ui.button("Close").clicked() {
                                close = true;
                            }
                        });
                    }
                    SendOutcome::Failed {
                        name,
                        message,
                        hint,
                        recovery_id,
                    } => {
                        ui.colored_label(ACCENT_RED, format!("{name} was not sent"));
                        ui.label(message.as_str());
                        if let Some(hint) = hint {
                            ui.label(RichText::new(hint.as_str()).color(TEXT_MUTED));
                        }
                        ui.horizontal(|ui| {
                            if let Some(id) = recovery_id {
                                if ui.button("Open Advanced").clicked() {
                                    event = Some(SendEvent::OpenAdvanced(id.clone()));
                                    close = true;
                                }
                            }
                            if ui.button("Close").clicked() {
                                close = true;
                            }
                        });
                    }
                });
            }
        }
        if close {
            if let SendStage::ConfirmReplace { plan, .. } = &self.stage {
                event = Some(SendEvent::CancelReplacement(plan.operation_id.clone()));
            }
            self.stage = SendStage::Idle;
        }
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> SendRequest {
        SendRequest {
            tab: TabId(1),
            path: "MYGAME.hpprgm".into(),
            name: "MYGAME".into(),
            source: "EXPORT MYGAME() BEGIN END;".into(),
            sha256: payload_sha256("EXPORT MYGAME() BEGIN END;"),
        }
    }

    fn upload(state: &str) -> ppl_programs::UploadResult {
        serde_json::from_value(serde_json::json!({
            "operation_id": "ZZT_U1", "state": state, "name": "MYGAME",
            "device": {"serial_id":"s","usb_pid":5441,"firmware_build":15157,"boot_version":null},
            "payload_sha256": "h", "stored_sha256": null, "comparison": null, "error": null
        }))
        .unwrap()
    }

    #[test]
    fn suggested_names_are_valid_calculator_names() {
        for (path, want) in [
            ("dir/MyGame.hpprgm", "MyGame"),
            ("2048 game.ppl", "P2048_game"),
            ("a-very-long-program-name.ppl", "a_very_long_prog"),
        ] {
            let name = suggested_name(std::path::Path::new(path));
            assert_eq!(name, want);
            assert!(ppl_device::validate_program_name(&name).is_ok(), "{name}");
        }
    }

    #[test]
    fn verified_create_finishes_as_sent() {
        let mut flow = SendFlow {
            stage: SendStage::Sending(request()),
        };
        flow.on_result(Ok(JobOutput::Created(upload("upload_verified"))));
        assert!(matches!(
            flow.stage,
            SendStage::Finished(SendOutcome::Sent {
                replaced: false,
                ..
            })
        ));
    }

    #[test]
    fn unverified_create_offers_recovery() {
        let mut flow = SendFlow {
            stage: SendStage::Sending(request()),
        };
        flow.on_result(Ok(JobOutput::Created(upload("indeterminate"))));
        assert!(matches!(
            &flow.stage,
            SendStage::Finished(SendOutcome::Failed { recovery_id: Some(id), .. }) if id == "ZZT_U1"
        ));
    }

    #[test]
    fn device_errors_keep_their_hint() {
        let mut flow = SendFlow {
            stage: SendStage::Sending(request()),
        };
        flow.on_result(Err(JobError::from(ppl_device::DeviceError::NotFound)));
        assert!(matches!(
            &flow.stage,
            SendStage::Finished(SendOutcome::Failed { hint: Some(_), .. })
        ));
    }

    #[test]
    fn replace_result_requires_completed_phase() {
        let mut flow = SendFlow {
            stage: SendStage::Replacing(request()),
        };
        flow.on_result(Ok(JobOutput::Replaced(
            serde_json::json!({"operation_id":"ZZT_D1","phase":"unresolved"}),
        )));
        assert!(matches!(
            &flow.stage,
            SendStage::Finished(SendOutcome::Failed { recovery_id: Some(id), .. }) if id == "ZZT_D1"
        ));
        let mut flow = SendFlow {
            stage: SendStage::Replacing(request()),
        };
        flow.on_result(Ok(JobOutput::Replaced(
            serde_json::json!({"operation_id":"ZZT_D1","phase":"completed"}),
        )));
        assert!(matches!(
            flow.stage,
            SendStage::Finished(SendOutcome::Sent { replaced: true, .. })
        ));
    }

    #[test]
    fn late_results_after_cancel_are_ignored() {
        let mut flow = SendFlow::default();
        flow.on_result(Ok(JobOutput::Created(upload("upload_verified"))));
        assert!(matches!(flow.stage, SendStage::Idle));
    }

    #[test]
    fn closed_flow_starts_nothing() {
        let mut flow = SendFlow::default();
        assert_eq!(flow.show(&egui::Context::default()), None);
        assert!(!flow.active());
    }

    #[test]
    fn hash_matches_the_guarded_service() {
        let source = "EXPORT DEMO() BEGIN END;";
        let image = ppl_programs::ProgramImage::new(
            "DEMO",
            ppl_programs::Source::text(source, "DEMO").unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload_sha256(source),
            ppl_engine::source::hash(&image.payload())
        );
    }
}
