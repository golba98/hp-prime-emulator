//! Physical management controls. All USB and journal work runs on a worker;
//! mutations require a separately clicked, content-bound confirmation.
use ppl_device::control::{ManagementAction as Action, Request};
use ppl_programs::{ProgramImage, Source, delete_store::Store};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::mpsc::{self, Receiver},
};

#[derive(Default)]
pub struct ProgramManager {
    pub open: bool,
    pub device: String,
    pub socket: String,
    pub name: String,
    pub recovery_id: String,
    pub status: String,
    pending: Option<(Action, String)>,
    pending_device: String,
    job: Option<Receiver<Result<Value, String>>>,
    preview: Option<Value>,
}
impl ProgramManager {
    fn start(&mut self, action: Option<Action>, ctx: &egui::Context) {
        if self.job.is_some() {
            return;
        }
        let selected = self.device.clone();
        let socket = self.socket.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        self.job = Some(rx);
        self.status = "Device operation in progress…".into();
        std::thread::spawn(move || {
            let result = (|| -> Result<Value, String> {
                if !socket.is_empty() {
                    let request = match action {
                        Some(action) => {
                            if selected.is_empty() {
                                return Err("Select the device serial ID before management through an owner socket".into());
                            }
                            Request::Manage {
                                device_serial_id: selected,
                                action,
                            }
                        }
                        None => Request::Status,
                    };
                    let response = ppl_device::control::call(Path::new(&socket), &request)
                        .map_err(|e| e.to_string())?;
                    if !response.report["error"].is_null() {
                        return Err(response.report["error"].to_string());
                    }
                    return Ok(if response.report["operation"] == "program_management" {
                        response.report["result"].clone()
                    } else {
                        response.report
                    });
                }
                let mut prime = ppl_programs::open_selected(
                    (!selected.is_empty()).then_some(selected.as_str()),
                )
                .map_err(|e| e.to_string())?;
                let identity =
                    ppl_programs::DeviceIdentity::read(&mut prime).map_err(|e| e.to_string())?;
                match action {
                    None => Ok(
                        json!({"device":identity,"state":"connected","physical_execution":"unsupported"}),
                    ),
                    Some(action) => ppl_programs::service::handle(
                        &mut prime,
                        &identity.serial_id,
                        &action,
                        &Store::new(Path::new("artifacts/delete")),
                    )
                    .map_err(|e| e.to_string()),
                }
            })();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn poll(&mut self) {
        let result = self.job.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(result) = result {
            self.job = None;
            match result {
                Err(error) => {
                    self.status = format!("Device operation stopped: {error}");
                    self.pending = None;
                }
                Ok(value) => {
                    if let Some(id) = value["device"]["serial_id"]
                        .as_str()
                        .or_else(|| value["device_serial_id"].as_str())
                    {
                        self.device = id.into();
                    }
                    if let Some(id) = value["operation_id"]
                        .as_str()
                        .or_else(|| value["id"].as_str())
                    {
                        self.recovery_id = id.into();
                    }
                    self.status = value["state"]
                        .as_str()
                        .or_else(|| value["phase"].as_str())
                        .unwrap_or("Preparation complete; review the operation below")
                        .replace('_', " ");
                    if value.get("new_sha256").is_some() {
                        self.pending = Some((
                            Action::ApplyReplacement {
                                operation_id: self.recovery_id.clone(),
                                confirmation_new_sha256: value["new_sha256"]
                                    .as_str()
                                    .unwrap_or("")
                                    .into(),
                            },
                            format!(
                                "Replace program {} on device {}. Old hash: {}. New hash: {}. This deletes then creates; the backup is retained.",
                                value["name"],
                                self.device,
                                value["old_sha256"],
                                value["new_sha256"]
                            ),
                        ));
                    } else if value["state"] == "prepared" && value["targets"].is_array() {
                        self.pending = Some((
                            Action::ApplyDelete {
                                operation_id: self.recovery_id.clone(),
                                confirmation_old_sha256: value["targets"][0]["sha256"]
                                    .as_str()
                                    .unwrap_or("")
                                    .into(),
                            },
                            format!(
                                "Delete exactly program {} on device {}. Backup hash: {}.",
                                value["targets"][0]["name"],
                                self.device,
                                value["targets"][0]["sha256"]
                            ),
                        ));
                    }
                    if self.pending.is_some() {
                        self.pending_device = self.device.clone();
                    }
                    self.preview = Some(value);
                }
            }
        }
    }
    pub fn show(&mut self, ctx: &egui::Context, source: Option<(&str, &str)>) {
        self.poll();
        let expected = self.pending.as_ref().and_then(|(action, _)| match action {
            Action::Create {
                confirmation_sha256,
                ..
            } => Some(confirmation_sha256),
            Action::ApplyReplacement {
                confirmation_new_sha256,
                ..
            } => Some(confirmation_new_sha256),
            _ => None,
        });
        if let Some(expected) = expected {
            let current = source.map(|(_, text)| {
                ppl_engine::source::hash(
                    &text
                        .encode_utf16()
                        .flat_map(u16::to_le_bytes)
                        .collect::<Vec<_>>(),
                )
            });
            if current.as_ref() != Some(expected) {
                self.pending = None;
                self.status = "Editor source changed; prepare and review the upload again".into();
            }
        }
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("Physical calculator programs").open(&mut open).default_width(580.).show(ctx,|ui|{
            ui.label("Uploads, source reads, and deletions use the shared guarded manager. Physical Check and Run are unsupported.");
            ui.add_enabled_ui(self.job.is_none(),|ui|{
                ui.horizontal(|ui|{ui.label("Device serial ID");ui.text_edit_singleline(&mut self.device);if ui.button("Connect / status").clicked(){self.pending=None;self.start(None,ctx);}});
                ui.horizontal(|ui|{ui.label("Exclusive owner socket (optional)");ui.text_edit_singleline(&mut self.socket);});
                ui.horizontal(|ui|{ui.label("Exact program name");ui.text_edit_singleline(&mut self.name);});
                let image=source.map(|(title,text)|Source::text(text,title).map_err(ppl_programs::refused).and_then(|s|ProgramImage::new(&self.name,s)));
                if let Some(Ok(image))=&image {
                    ui.label(format!("{} UTF-16 payload bytes; {} message bytes; {} reports",image.sizes.payload_bytes,image.sizes.object_message_bytes,image.sizes.hid_reports));
                    if !image.sizes.supported{ui.label("Upload blocked: above the evidenced legacy transfer ceiling; its cause remains unresolved.");}
                }
                ui.horizontal(|ui|{
                    let supported=matches!(&image,Some(Ok(image)) if image.sizes.supported);
                    if ui.add_enabled(supported,egui::Button::new("Prepare new upload")).clicked(){
                        if let Some(Ok(image))=&image {
                            let hash=ppl_engine::source::hash(&image.payload());
                            self.pending_device=self.device.clone();
                            self.pending=Some((Action::Create{name:image.name.clone(),source:image.source.original_text.clone(),confirmation_sha256:hash.clone()},format!("Create new program {} on device {} from source hash {}. An existing destination will be refused.",image.name,self.device,hash)));
                        }
                    }
                    if ui.add_enabled(supported,egui::Button::new("Prepare replacement")).clicked(){if let Some(Ok(image))=&image {self.pending=None;self.start(Some(Action::PrepareReplacement{name:image.name.clone(),source:image.source.original_text.clone()}),ctx);}}
                    if ui.button("Prepare deletion").clicked(){self.pending=None;self.start(Some(Action::PrepareDelete{name:self.name.clone()}),ctx);}
                });
                ui.horizontal(|ui| {
                    ui.label("Recovery operation ID");
                    ui.text_edit_singleline(&mut self.recovery_id);
                });
                ui.horizontal_wrapped(|ui| {
                    for (label, action) in [
                        ("Verify replacement", Action::VerifyReplacement { operation_id: self.recovery_id.clone() }),
                        ("Verify deletion", Action::VerifyDelete { operation_id: self.recovery_id.clone() }),
                        ("Verify upload", Action::VerifyUpload { operation_id: self.recovery_id.clone() }),
                    ] {
                        if ui.button(label).clicked() { self.pending = None; self.start(Some(action), ctx); }
                    }
                });
                if let Some((_,description))=&self.pending{ui.separator();ui.label(description);}
                if self.pending.is_some(){ui.horizontal(|ui|{
                    if ui.button("Authorize this exact operation").clicked(){let (action,_)=self.pending.take().unwrap();if self.device.is_empty() || self.device!=self.pending_device {self.status="Device selection changed; prepare and review the operation again".into();}else{self.start(Some(action),ctx);}}
                    if ui.button("Cancel").clicked(){self.pending=None;}
                });}
            });
            if self.job.is_some(){ui.spinner();}
            ui.label(&self.status);
            if let Some(value)=&self.preview{egui::CollapsingHeader::new("Operation evidence").show(ui,|ui|{ui.label(serde_json::to_string_pretty(value).unwrap_or_default());});}
        });
        self.open = open;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_manager_never_starts_usb_or_authorizes_mutation() {
        let mut manager = ProgramManager::default();
        manager.show(
            &egui::Context::default(),
            Some(("test", "EXPORT DEMO() BEGIN END;")),
        );
        assert!(manager.job.is_none());
        assert!(manager.pending.is_none());
    }
    #[test]
    fn result_requires_separate_confirmation_for_exact_deletion() {
        let (tx, rx) = mpsc::channel();
        let mut manager = ProgramManager {
            job: Some(rx),
            ..Default::default()
        };
        tx.send(Ok(json!({"id":"ZZT_D12345","state":"prepared","targets":[{"name":"ZZT_TEST","sha256":"abc"}]}))).unwrap();
        manager.poll();
        assert!(
            matches!(manager.pending,Some((Action::ApplyDelete{operation_id,..},_)) if operation_id=="ZZT_D12345")
        );
        assert!(manager.job.is_none());
    }
    #[test]
    fn edited_source_invalidates_pending_upload_without_starting_usb() {
        let mut manager = ProgramManager::default();
        let original = "EXPORT DEMO() BEGIN RETURN 1; END;";
        let bytes: Vec<u8> = original.encode_utf16().flat_map(u16::to_le_bytes).collect();
        manager.pending = Some((
            Action::Create {
                name: "DEMO".into(),
                source: original.into(),
                confirmation_sha256: ppl_engine::source::hash(&bytes),
            },
            "Review source".into(),
        ));
        manager.show(
            &egui::Context::default(),
            Some(("DEMO", "changed after review")),
        );
        assert!(manager.pending.is_none());
        assert!(manager.job.is_none());
    }
}
