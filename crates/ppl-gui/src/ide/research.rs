//! Research discovery stays separate from calculator/PPL execution selection.
use ppl_engine::full_system::{self, BackendDescriptor};
use std::sync::mpsc::{self, Receiver, TryRecvError};
#[derive(Debug, Default)]
pub struct ResearchPanel {
    pub open: bool,
    pending: Option<(String, Receiver<BackendDescriptor>)>,
    result: Option<(String, BackendDescriptor)>,
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
    pub fn show(&mut self, ctx: &egui::Context, build_root: &mut String) {
        self.poll(build_root);
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
            ui.label("Run self-authored hardware cases using `ppl full-system run`; inspect `ppl full-system --help` for options.");
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
