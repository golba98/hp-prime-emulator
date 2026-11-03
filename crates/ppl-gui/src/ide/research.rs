//! Research discovery stays separate from calculator/PPL execution selection.
use ppl_engine::full_system::{self, BackendDescriptor};
use std::sync::mpsc::{self, Receiver, TryRecvError};
#[derive(Default)]
pub struct ResearchPanel {
    pub open: bool,
    pending: Option<(String, Receiver<BackendDescriptor>)>,
    result: Option<(String, BackendDescriptor)>,
    selected_case: usize,
    running: Option<ResearchJob>,
    execution: Option<String>,
    execution_source: String,
    summary: Option<String>,
    preview_path: Option<std::path::PathBuf>,
    preview: Option<egui::TextureHandle>,
}
impl std::fmt::Debug for ResearchPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResearchPanel")
            .field("open", &self.open)
            .field("running", &self.running.is_some())
            .finish_non_exhaustive()
    }
}
#[derive(Debug)]
struct ResearchJob {
    source: String,
    cancel: ppl_engine::Cancellation,
    receiver: Receiver<Result<serde_json::Value, String>>,
}
impl Drop for ResearchJob {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl ResearchPanel {
    fn poll(&mut self, path: &str) {
        if self
            .result
            .as_ref()
            .is_some_and(|(source, _)| source != path)
        {
            self.result = None;
        }
        let Some((source, receiver)) = &self.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(report) => Some(report),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                return;
            }
        };
        if source == path {
            self.result = result.map(|report| (source.clone(), report));
        }
        self.pending = None;
    }
    fn poll_execution(&mut self, build_root: &str) {
        if self.execution_source != build_root {
            self.execution = None;
            self.summary = None;
            self.preview = None;
            self.preview_path = None;
            self.execution_source = build_root.to_owned();
        }
        if let Some(job) = &self.running {
            if job.source != build_root || !self.open {
                job.cancel.cancel();
            }
            match job.receiver.try_recv() {
                Ok(result) => {
                    if job.source == build_root {
                        self.execution = Some(match result {
                            Ok(value) => {
                                self.summary = Some(format!(
                                    "{}: {}",
                                    value["case"].as_str().unwrap_or("Hardware test"),
                                    value["status"].as_str().unwrap_or("unknown")
                                ));
                                self.preview_path = value["output_directory"]
                                    .as_str()
                                    .filter(|_| {
                                        value["case"] == "lcd" && value["status"] == "passed"
                                    })
                                    .map(|root| std::path::Path::new(root).join("run/frame1.png"));
                                serde_json::to_string_pretty(&value).unwrap_or_default()
                            }
                            Err(error) => {
                                self.summary = Some(format!("Hardware test failed: {error}"));
                                error
                            }
                        });
                    }
                    self.running = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.execution = Some("Research worker disconnected".into());
                    self.running = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
    }
    pub(super) fn execution_result(&self) -> Option<&str> {
        self.execution.as_deref()
    }
    pub(super) fn start_case(&mut self, ctx: &egui::Context, build_root: &str, case: &str) {
        if let Some(index) = full_system::CASES.iter().position(|name| *name == case) {
            self.selected_case = index;
            self.start(ctx, build_root);
        }
    }
    fn start(&mut self, ctx: &egui::Context, build_root: &str) {
        if self.running.is_some() {
            return;
        }
        let source = build_root.to_owned();
        let root = std::path::PathBuf::from(&source);
        let output = std::env::temp_dir().join(format!(
            "prime-research-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let case = full_system::CASES[self.selected_case];
        let cancel = ppl_engine::Cancellation::default();
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        let context = ctx.clone();
        std::thread::spawn(move || {
            let result = full_system::execute(&root, case, &output, 30, &worker_cancel);
            let _ = sender.send(result.map(|mut value| {
                value["output_directory"] = serde_json::json!(output);
                value
            }));
            context.request_repaint();
        });
        self.running = Some(ResearchJob {
            source,
            cancel,
            receiver,
        });
        self.execution = None;
        self.summary = None;
        self.preview = None;
        self.preview_path = None;
    }
    pub fn show(&mut self, ctx: &egui::Context, build_root: &mut String) {
        self.poll(build_root);
        self.poll_execution(build_root);
        if let Some(path) = self.preview_path.take() {
            if full_system::regular_digest(&path, 1024 * 1024).is_ok() {
                if let Ok(bytes) = std::fs::read(path) {
                    if bytes.len() >= 24
                        && bytes[..8] == *b"\x89PNG\r\n\x1a\n"
                        && bytes[16..24] == [0, 0, 1, 64, 0, 0, 0, 240]
                    {
                        if let Ok(grob) = ppl_graphics::Grob::read_png(&bytes) {
                            let pixels = grob
                                .pixels()
                                .iter()
                                .map(|p| {
                                    egui::Color32::from_rgb(
                                        (p >> 16) as u8,
                                        (p >> 8) as u8,
                                        *p as u8,
                                    )
                                })
                                .collect();
                            self.preview = Some(ctx.load_texture(
                                "research_frame",
                                egui::ColorImage {
                                    size: [320, 240],
                                    pixels,
                                    source_size: egui::vec2(320.0, 240.0),
                                },
                                egui::TextureOptions::NEAREST,
                            ));
                        }
                    }
                }
            }
        }
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("FullSystem hardware diagnostics").open(&mut open).default_width(550.0).show(ctx,|ui| {
            ui.label("Isolated hp-prime ARM926 research backend");
            ui.label("Synthetic hardware tests; calculator kernel boot and PPL execution are unavailable.");
            ui.label("Pinned research build directory");
            ui.add(egui::TextEdit::singleline(build_root).desired_width(f32::INFINITY));
            if ui.add_enabled(self.pending.is_none(),egui::Button::new("Validate research build")).clicked() {
                let path=build_root.clone();let worker_path=path.clone();let (sender,receiver)=mpsc::channel();let context=ctx.clone();
                std::thread::spawn(move || {
                    let report=full_system::inspect((!worker_path.trim().is_empty()).then(||std::path::Path::new(&worker_path)));
                    let _=sender.send(report);context.request_repaint();
                });
                self.pending=Some((path,receiver));self.result=None;
            }
            if self.pending.is_some() {ui.spinner();}
            if let Some((source,report))=&self.result { if source==build_root {
                ui.label(format!("Resources: {}",report.availability));
                if let Some(reason)=&report.reason {ui.label(reason);}
                if let Some(identity)=&report.resource_identity {ui.monospace(identity);}
                ui.label("Execution preflight remains pending until a diagnostic runs.");
            }}
            ui.separator();
            let cases = full_system::CASES;
            egui::ComboBox::from_id_salt("research_case").selected_text(cases[self.selected_case]).show_ui(ui, |ui| {
                for (index, case) in cases.iter().enumerate() {
                    ui.selectable_value(&mut self.selected_case, index, *case);
                }
            });
            if ui.add_enabled(self.running.is_none() && !build_root.trim().is_empty(), egui::Button::new("Run hardware test")).clicked() {
                self.start(ctx, build_root);
            }
            if let Some(job) = &self.running {
                ui.spinner();
                if ui.button("Cancel hardware test").clicked() { job.cancel.cancel(); }
            }
            if let Some(summary) = &self.summary { ui.strong(summary); }
            if let Some(texture) = &self.preview { ui.image((texture.id(), egui::vec2(320.0,240.0))); }
            if let Some(text) = &self.execution {
                ui.collapsing("UART, registers and full result", |ui| {
                    egui::ScrollArea::both().max_height(260.0).show(ui, |ui| { ui.monospace(text); });
                });
            }
            ui.label("PPL Run/Check, CAS, graphing and calculator UI remain on the Native/G1 Firmware modes.");
        });
        self.open = open;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_resource_checks_cannot_replace_current_path() {
        let (sender, receiver) = mpsc::channel();
        sender.send(full_system::inspect(None)).unwrap();
        let mut panel = ResearchPanel {
            pending: Some(("old".into(), receiver)),
            ..Default::default()
        };
        panel.poll("new");
        assert!(panel.result.is_none());
        assert!(panel.pending.is_none());
    }
    #[test]
    fn edits_invalidate_prior_result_and_closed_worker_does_not_stick() {
        let (sender, receiver) = mpsc::channel();
        drop(sender);
        let mut panel = ResearchPanel {
            pending: Some(("old".into(), receiver)),
            result: Some(("old".into(), full_system::inspect(None))),
            ..Default::default()
        };
        panel.poll("new");
        assert!(panel.pending.is_none());
        assert!(panel.result.is_none());
    }
    #[test]
    fn changing_build_cancels_worker_and_discards_stale_completion() {
        let (sender, receiver) = mpsc::channel();
        let cancel = ppl_engine::Cancellation::default();
        let mut panel = ResearchPanel {
            open: true,
            running: Some(ResearchJob {
                source: "old".into(),
                cancel: cancel.clone(),
                receiver,
            }),
            ..Default::default()
        };
        panel.poll_execution("new");
        assert!(cancel.cancelled());
        sender
            .send(Ok(serde_json::json!({"status":"passed"})))
            .unwrap();
        panel.poll_execution("new");
        assert!(panel.running.is_none());
        assert!(panel.execution.is_none());
    }
    #[test]
    fn closing_or_dropping_panel_cancels_running_job() {
        let (_sender, receiver) = mpsc::channel();
        let cancel = ppl_engine::Cancellation::default();
        let mut panel = ResearchPanel {
            running: Some(ResearchJob {
                source: "build".into(),
                cancel: cancel.clone(),
                receiver,
            }),
            ..Default::default()
        };
        panel.poll_execution("build");
        assert!(cancel.cancelled());
        let (_sender, receiver) = mpsc::channel();
        let cancel = ppl_engine::Cancellation::default();
        panel.running = Some(ResearchJob {
            source: "build".into(),
            cancel: cancel.clone(),
            receiver,
        });
        drop(panel);
        assert!(cancel.cancelled());
    }
    #[test]
    fn research_configuration_persists_without_changing_ppl_mode() {
        let state = super::super::IdeAppState {
            full_system_build_root: "/private/research build".into(),
            ..Default::default()
        };
        let persisted = state.to_persistent();
        let mut restored = super::super::IdeAppState::default();
        restored.apply_persistent(&persisted);
        assert_eq!(
            restored.full_system_build_root,
            state.full_system_build_root
        );
        assert!(!restored.firmware_backend);
    }
}
